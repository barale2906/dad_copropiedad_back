//! Puente de acceso en sitio (plan 05, fase 3): un binario que corre en un mini PC de la PH,
//! habla con los dispositivos por su adaptador ([`ControladorDispositivo`]) y con la nube por
//! HTTPS saliente con su token de máquina.
//!
//! **Decisión en el borde, verdad en la nube**: la portería no se queda sin acceso si se cae
//! internet. El puente decide con su [`Cache`] (credenciales por huella, ventanas, visitas con
//! QR) y guarda los eventos en una [`ColaEventos`] que reporta cuando vuelve la conexión.
//!
//! - Las credenciales llegan por huella (`domain::acceso::huella_credencial`): el puente
//!   calcula la huella de lo leído y nunca guarda el identificador en claro.
//! - Los QR temporales se validan con la clave pública Ed25519 de la PH, sin consultar la nube;
//!   los usos se cuentan localmente y se reconcilian con los del servidor.
//! - Una credencial revocada sale de la cache en la siguiente sincronización (por defecto cada
//!   20 s; invariante 1 del plan: menos de un minuto).
//! - Las plantillas biométricas nunca salen del equipo: cuando la nube pide borrarlas, el
//!   adaptador las borra y el puente confirma.

pub mod cliente;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use domain::{
    CargaQr, CredencialId, Horario, PuntoAccesoId, ResultadoAcceso, SentidoAcceso, TenantId,
    TipoCredencial, VentanaAcceso, VisitaId, acceso::huella_credencial,
};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Datos que entrega la nube (espejo de `GET /api/v1/puentes/sincronizacion`)
// ---------------------------------------------------------------------------------------------

/// Ventana en la que vale una credencial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ventana {
    pub puntos: Vec<Uuid>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub horario: Option<Horario>,
}

impl Ventana {
    fn dominio(&self) -> VentanaAcceso {
        VentanaAcceso {
            puntos: self
                .puntos
                .iter()
                .copied()
                .map(PuntoAccesoId::from_uuid)
                .collect(),
            vigente_desde: self.vigente_desde,
            vigente_hasta: self.vigente_hasta,
            horario: self.horario.clone(),
        }
    }
}

/// Credencial sincronizada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredencialRemota {
    pub id: Uuid,
    pub tipo: TipoCredencial,
    pub huella: String,
    pub activa: bool,
    pub vigente_hasta: Option<DateTime<Utc>>,
    #[serde(default)]
    pub ventanas: Vec<Ventana>,
    #[serde(default)]
    pub borrar_plantilla: bool,
}

/// Visita pre-autorizada sincronizada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitaRemota {
    pub id: Uuid,
    pub activa: bool,
    pub usos: u16,
}

/// Respuesta de la sincronización (solo lo que usa el puente).
#[derive(Debug, Clone, Deserialize)]
pub struct Sincronizacion {
    pub copropiedad_id: Uuid,
    #[serde(default)]
    pub credenciales: Vec<CredencialRemota>,
    #[serde(default)]
    pub visitas: Vec<VisitaRemota>,
    pub cursor: i64,
    pub clave_publica_qr: String,
}

// ---------------------------------------------------------------------------------------------
// Cache local
// ---------------------------------------------------------------------------------------------

/// Entrada de la cache (sin el identificador: solo la huella como clave).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrada {
    pub id: Uuid,
    pub tipo: TipoCredencial,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub ventanas: Vec<Ventana>,
}

/// Estado de una visita en la cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitaCache {
    pub activa: bool,
    /// Usos según el servidor.
    pub usos_servidor: u16,
    /// Usos registrados aquí desde la última sincronización.
    pub usos_locales: u16,
}

/// Lo último sincronizado. Se guarda en disco para decidir aunque el puente se reinicie sin
/// internet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub tenant: Option<Uuid>,
    pub cursor: i64,
    /// Clave pública Ed25519 de la PH en hex.
    pub clave_publica: Option<String>,
    pub credenciales: HashMap<String, Entrada>,
    pub visitas: HashMap<Uuid, VisitaCache>,
}

/// Error de la cache en disco.
#[derive(Debug, thiserror::Error)]
pub enum ErrorDisco {
    #[error("no se pudo leer o escribir {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("archivo corrupto {0}: {1}")]
    Formato(PathBuf, serde_json::Error),
}

fn leer_json<T: for<'de> Deserialize<'de> + Default>(ruta: &Path) -> Result<T, ErrorDisco> {
    match std::fs::read(ruta) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| ErrorDisco::Formato(ruta.into(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(ErrorDisco::Io(ruta.into(), e)),
    }
}

/// Escritura atómica: archivo temporal + `rename` (un corte de luz no deja la cache a medias).
fn escribir_json<T: Serialize>(ruta: &Path, v: &T) -> Result<(), ErrorDisco> {
    let tmp = ruta.with_extension("tmp");
    let datos = serde_json::to_vec(v).map_err(|e| ErrorDisco::Formato(ruta.into(), e))?;
    std::fs::write(&tmp, datos).map_err(|e| ErrorDisco::Io(tmp.clone(), e))?;
    std::fs::rename(&tmp, ruta).map_err(|e| ErrorDisco::Io(ruta.into(), e))
}

impl Cache {
    /// # Errors
    ///
    /// [`ErrorDisco`] si el archivo existe pero no se puede leer.
    pub fn cargar(ruta: &Path) -> Result<Self, ErrorDisco> {
        leer_json(ruta)
    }

    /// # Errors
    ///
    /// [`ErrorDisco`] si no se puede escribir.
    pub fn guardar(&self, ruta: &Path) -> Result<(), ErrorDisco> {
        escribir_json(ruta, self)
    }

    /// Aplica los cambios de la nube. Devuelve las credenciales biométricas cuya plantilla hay
    /// que borrar del equipo.
    pub fn aplicar(&mut self, s: Sincronizacion) -> Vec<CredencialId> {
        if self.tenant != Some(s.copropiedad_id) {
            // Otro tenant (puente reasignado): se empieza de cero.
            *self = Self {
                tenant: Some(s.copropiedad_id),
                ..Self::default()
            };
        }
        self.clave_publica = Some(s.clave_publica_qr);
        let mut borrar = Vec::new();
        for c in s.credenciales {
            self.credenciales.retain(|_, e| e.id != c.id);
            if c.borrar_plantilla {
                borrar.push(CredencialId::from_uuid(c.id));
            }
            if c.activa {
                self.credenciales.insert(
                    c.huella,
                    Entrada {
                        id: c.id,
                        tipo: c.tipo,
                        vigente_hasta: c.vigente_hasta,
                        ventanas: c.ventanas,
                    },
                );
            }
        }
        for v in s.visitas {
            let e = self.visitas.entry(v.id).or_default();
            e.activa = v.activa;
            if v.usos >= e.usos_servidor {
                // Los usos locales ya reportados quedan contados en el servidor.
                e.usos_locales = e.usos_locales.saturating_sub(v.usos - e.usos_servidor);
                e.usos_servidor = v.usos;
            }
        }
        self.cursor = self.cursor.max(s.cursor);
        borrar
    }
}

// ---------------------------------------------------------------------------------------------
// Decisión sin internet
// ---------------------------------------------------------------------------------------------

/// Lo que leyó un dispositivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lectura {
    /// Tarjeta, PIN, placa (LPR), id biométrico o QR permanente.
    Credencial { tipo: TipoCredencial, valor: String },
    /// Texto de un QR (si es un QR de visita se valida su firma).
    Qr(String),
}

/// Decisión del puente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub resultado: ResultadoAcceso,
    pub motivo: &'static str,
    pub credencial: Option<CredencialId>,
    pub visita: Option<VisitaId>,
}

impl Decision {
    fn denegar(motivo: &'static str) -> Self {
        Self {
            resultado: ResultadoAcceso::Denegado,
            motivo,
            credencial: None,
            visita: None,
        }
    }
}

fn clave(cache: &Cache) -> Option<[u8; 32]> {
    let b = hex::decode(cache.clave_publica.as_deref()?).ok()?;
    b.try_into().ok()
}

/// Decide con la cache (sin red). Si permite un QR de visita, cuenta el uso localmente.
pub fn decidir(
    cache: &mut Cache,
    punto: PuntoAccesoId,
    lectura: &Lectura,
    ahora: DateTime<Utc>,
    tz: Tz,
) -> Decision {
    let Some(tenant) = cache.tenant.map(TenantId::from_uuid) else {
        return Decision::denegar("puente sin sincronizar");
    };
    let (tipo, valor) = match lectura {
        Lectura::Qr(texto) => {
            if let Some((carga, firma)) = CargaQr::leer_texto(texto) {
                return decidir_qr(cache, tenant, &carga, &firma, ahora);
            }
            (TipoCredencial::QrPermanente, texto.as_str())
        }
        Lectura::Credencial { tipo, valor } => (*tipo, valor.as_str()),
    };
    let Ok(normalizado) = tipo.normalizar(valor) else {
        return Decision::denegar("lectura inválida");
    };
    let Some(e) = cache
        .credenciales
        .get(&huella_credencial(tenant, tipo, &normalizado))
    else {
        return Decision::denegar("credencial desconocida");
    };
    let credencial = Some(CredencialId::from_uuid(e.id));
    if e.vigente_hasta.is_some_and(|h| h <= ahora) {
        return Decision {
            credencial,
            ..Decision::denegar("credencial vencida")
        };
    }
    if e.ventanas
        .iter()
        .any(|v| v.dominio().permite(punto, ahora, tz))
    {
        Decision {
            resultado: ResultadoAcceso::Permitido,
            motivo: "autorizado",
            credencial,
            visita: None,
        }
    } else {
        Decision {
            credencial,
            ..Decision::denegar("sin autorización en este punto u horario")
        }
    }
}

fn decidir_qr(
    cache: &mut Cache,
    tenant: TenantId,
    carga: &CargaQr,
    firma: &[u8],
    ahora: DateTime<Utc>,
) -> Decision {
    let Some(clave) = clave(cache) else {
        return Decision::denegar("sin clave pública de la PH");
    };
    let firma_valida = carga.tenant == tenant
        && UnparsedPublicKey::new(&ED25519, clave)
            .verify(&carga.bytes(), firma)
            .is_ok();
    if !firma_valida {
        return Decision::denegar("QR inválido");
    }
    let visita = Some(carga.visita);
    // Una visita aún no sincronizada (creada hace segundos) vale por su firma.
    let estado = cache
        .visitas
        .entry(carga.visita.as_uuid())
        .or_insert(VisitaCache {
            activa: true,
            usos_servidor: 0,
            usos_locales: 0,
        });
    if !estado.activa {
        return Decision {
            visita,
            ..Decision::denegar("visita cancelada o terminada")
        };
    }
    let usados = u32::from(estado.usos_servidor) + u32::from(estado.usos_locales);
    if !carga.admite(ahora, usados) {
        return Decision {
            visita,
            ..Decision::denegar("QR vencido o sin usos")
        };
    }
    estado.usos_locales += 1;
    Decision {
        resultado: ResultadoAcceso::Permitido,
        motivo: "visita autorizada",
        credencial: None,
        visita,
    }
}

// ---------------------------------------------------------------------------------------------
// Cola de eventos
// ---------------------------------------------------------------------------------------------

/// Evento pendiente de reportar (espejo de `POST /api/v1/puentes/eventos`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventoPendiente {
    pub id_local: String,
    pub dispositivo_id: Option<Uuid>,
    pub punto_acceso_id: Option<Uuid>,
    pub credencial_id: Option<Uuid>,
    pub visita_id: Option<Uuid>,
    pub sentido: SentidoAcceso,
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    pub placa: Option<String>,
    pub foto_base64: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Eventos y confirmaciones de borrado pendientes, persistidos en disco.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColaEventos {
    pub eventos: Vec<EventoPendiente>,
    pub borrados_biometricos: Vec<Uuid>,
}

/// Eventos por reporte (límite de la API).
pub const LOTE_REPORTE: usize = 500;

impl ColaEventos {
    /// # Errors
    ///
    /// [`ErrorDisco`] si el archivo existe pero no se puede leer.
    pub fn cargar(ruta: &Path) -> Result<Self, ErrorDisco> {
        leer_json(ruta)
    }

    /// # Errors
    ///
    /// [`ErrorDisco`] si no se puede escribir.
    pub fn guardar(&self, ruta: &Path) -> Result<(), ErrorDisco> {
        escribir_json(ruta, self)
    }

    /// Registra la decisión como evento pendiente.
    pub fn anotar(
        &mut self,
        d: &Decision,
        punto: PuntoAccesoId,
        dispositivo: Option<Uuid>,
        sentido: SentidoAcceso,
        placa: Option<String>,
        ahora: DateTime<Utc>,
    ) {
        self.eventos.push(EventoPendiente {
            id_local: Uuid::now_v7().to_string(),
            dispositivo_id: dispositivo,
            punto_acceso_id: Some(punto.as_uuid()),
            credencial_id: d.credencial.map(CredencialId::as_uuid),
            visita_id: d.visita.map(VisitaId::as_uuid),
            sentido,
            resultado: d.resultado,
            motivo: Some(d.motivo.to_owned()),
            placa,
            foto_base64: None,
            ocurrido_en: ahora,
        });
    }

    /// Siguiente lote a reportar.
    pub fn lote(&self) -> (&[EventoPendiente], &[Uuid]) {
        (
            &self.eventos[..self.eventos.len().min(LOTE_REPORTE)],
            &self.borrados_biometricos,
        )
    }

    /// Quita lo que ya se reportó.
    pub fn confirmar(&mut self, eventos: usize, borrados: usize) {
        self.eventos.drain(..eventos.min(self.eventos.len()));
        self.borrados_biometricos
            .drain(..borrados.min(self.borrados_biometricos.len()));
    }
}

// ---------------------------------------------------------------------------------------------
// Dispositivos
// ---------------------------------------------------------------------------------------------

/// Lectura de un dispositivo con su contexto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LecturaDispositivo {
    pub dispositivo_id: Option<Uuid>,
    pub punto: PuntoAccesoId,
    pub sentido: SentidoAcceso,
    pub lectura: Lectura,
}

/// Adaptador de un fabricante (Hikvision, ZKTeco, Dahua, controladora Wiegand…). El primero se
/// implementa con el hardware de la primera PH; mientras tanto existe [`ControladorSimulado`].
pub trait ControladorDispositivo: Send {
    /// Siguiente lectura (`None` = no hay más: el controlador se cerró).
    fn leer(&mut self) -> impl std::future::Future<Output = Option<LecturaDispositivo>> + Send;

    /// Abre el torniquete, la talanquera o la puerta del punto.
    fn abrir(&mut self, l: &LecturaDispositivo) -> impl std::future::Future<Output = ()> + Send;

    /// Borra la plantilla biométrica del equipo (la nube revocó el consentimiento).
    fn borrar_plantilla(
        &mut self,
        credencial: CredencialId,
    ) -> impl std::future::Future<Output = bool> + Send;
}

/// Controlador de pruebas y demostración: recibe lecturas por un canal y registra lo que haría
/// el hardware.
pub struct ControladorSimulado {
    pub lecturas: tokio::sync::mpsc::Receiver<LecturaDispositivo>,
    pub aperturas: Vec<PuntoAccesoId>,
    pub plantillas_borradas: Vec<CredencialId>,
}

impl ControladorDispositivo for ControladorSimulado {
    async fn leer(&mut self) -> Option<LecturaDispositivo> {
        self.lecturas.recv().await
    }

    async fn abrir(&mut self, l: &LecturaDispositivo) {
        self.aperturas.push(l.punto);
        tracing::info!(punto = %l.punto, "apertura (simulada)");
    }

    async fn borrar_plantilla(&mut self, credencial: CredencialId) -> bool {
        self.plantillas_borradas.push(credencial);
        true
    }
}

/// Interpreta una línea del simulador: `<punto> <entrada|salida> <tipo|qr> <valor>`.
pub fn leer_linea_simulador(linea: &str) -> Option<LecturaDispositivo> {
    let mut partes = linea.split_whitespace();
    let punto = PuntoAccesoId::from_uuid(Uuid::try_parse(partes.next()?).ok()?);
    let sentido = SentidoAcceso::try_from(partes.next()?).ok()?;
    let tipo = partes.next()?;
    let valor: String = partes.collect::<Vec<_>>().join(" ");
    if valor.is_empty() {
        return None;
    }
    let lectura = if tipo == "qr" {
        Lectura::Qr(valor)
    } else {
        Lectura::Credencial {
            tipo: TipoCredencial::try_from(tipo).ok()?,
            valor,
        }
    };
    Some(LecturaDispositivo {
        dispositivo_id: None,
        punto,
        sentido,
        lectura,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    fn tz() -> Tz {
        chrono_tz::America::Bogota
    }

    fn ahora() -> DateTime<Utc> {
        DateTime::from_timestamp(1_794_000_000, 0).expect("fecha")
    }

    fn par() -> ring::signature::Ed25519KeyPair {
        ring::signature::Ed25519KeyPair::from_seed_unchecked(&[3; 32]).expect("par")
    }

    fn sincronizacion(tenant: Uuid, credenciales: Vec<CredencialRemota>) -> Sincronizacion {
        use ring::signature::KeyPair;
        Sincronizacion {
            copropiedad_id: tenant,
            credenciales,
            visitas: Vec::new(),
            cursor: 10,
            clave_publica_qr: hex::encode(par().public_key().as_ref()),
        }
    }

    fn tarjeta(tenant: Uuid, id: Uuid, activa: bool) -> CredencialRemota {
        CredencialRemota {
            id,
            tipo: TipoCredencial::TarjetaRfid,
            huella: huella_credencial(
                TenantId::from_uuid(tenant),
                TipoCredencial::TarjetaRfid,
                "0A1B2C3D",
            ),
            activa,
            vigente_hasta: None,
            ventanas: vec![Ventana {
                puntos: vec![],
                vigente_desde: ahora() - TimeDelta::days(1),
                vigente_hasta: None,
                horario: None,
            }],
            borrar_plantilla: false,
        }
    }

    fn leer_tarjeta() -> Lectura {
        Lectura::Credencial {
            tipo: TipoCredencial::TarjetaRfid,
            valor: "0a:1b:2c:3d".into(),
        }
    }

    #[test]
    fn decide_sin_internet_y_retira_lo_revocado() {
        let (tenant, id) = (Uuid::now_v7(), Uuid::now_v7());
        let mut cache = Cache::default();
        cache.aplicar(sincronizacion(tenant, vec![tarjeta(tenant, id, true)]));
        let punto = PuntoAccesoId::nuevo();

        let d = decidir(&mut cache, punto, &leer_tarjeta(), ahora(), tz());
        assert_eq!(d.resultado, ResultadoAcceso::Permitido);
        assert_eq!(d.credencial, Some(CredencialId::from_uuid(id)));

        cache.aplicar(sincronizacion(tenant, vec![tarjeta(tenant, id, false)]));
        let d = decidir(&mut cache, punto, &leer_tarjeta(), ahora(), tz());
        assert_eq!(
            d.resultado,
            ResultadoAcceso::Denegado,
            "revocada: sale de la cache"
        );
    }

    #[test]
    fn sin_sincronizar_niega_todo() {
        let d = decidir(
            &mut Cache::default(),
            PuntoAccesoId::nuevo(),
            &leer_tarjeta(),
            ahora(),
            tz(),
        );
        assert_eq!(
            (d.resultado, d.motivo),
            (ResultadoAcceso::Denegado, "puente sin sincronizar")
        );
    }

    fn qr(tenant: Uuid, usos: u16) -> (String, Uuid) {
        let visita = Uuid::now_v7();
        let carga = CargaQr::new(
            TenantId::from_uuid(tenant),
            VisitaId::from_uuid(visita),
            ahora() - TimeDelta::hours(1),
            ahora() + TimeDelta::hours(1),
            usos,
        )
        .expect("carga");
        (carga.texto(par().sign(&carga.bytes()).as_ref()), visita)
    }

    #[test]
    fn valida_qr_firmados_y_cuenta_usos_localmente() {
        let tenant = Uuid::now_v7();
        let mut cache = Cache::default();
        cache.aplicar(sincronizacion(tenant, vec![]));
        let (texto, visita) = qr(tenant, 1);
        let punto = PuntoAccesoId::nuevo();

        let d = decidir(
            &mut cache,
            punto,
            &Lectura::Qr(texto.clone()),
            ahora(),
            tz(),
        );
        assert_eq!(d.resultado, ResultadoAcceso::Permitido);
        assert_eq!(d.visita, Some(VisitaId::from_uuid(visita)));
        let d = decidir(&mut cache, punto, &Lectura::Qr(texto), ahora(), tz());
        assert_eq!(d.resultado, ResultadoAcceso::Denegado, "un solo uso");

        let (otro, _) = qr(Uuid::now_v7(), 1);
        let d = decidir(&mut cache, punto, &Lectura::Qr(otro), ahora(), tz());
        assert_eq!(d.motivo, "QR inválido", "de otra PH");
    }

    #[test]
    fn una_visita_cancelada_no_entra_aunque_el_qr_sea_valido() {
        let tenant = Uuid::now_v7();
        let mut cache = Cache::default();
        let (texto, visita) = qr(tenant, 3);
        let mut s = sincronizacion(tenant, vec![]);
        s.visitas.push(VisitaRemota {
            id: visita,
            activa: false,
            usos: 0,
        });
        cache.aplicar(s);

        let d = decidir(
            &mut cache,
            PuntoAccesoId::nuevo(),
            &Lectura::Qr(texto),
            ahora(),
            tz(),
        );

        assert_eq!(d.motivo, "visita cancelada o terminada");
    }

    #[test]
    fn la_cola_reporta_por_lotes_y_sobrevive_reinicios() {
        let dir = std::env::temp_dir().join(format!("puente-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("dir");
        let ruta = dir.join("cola.json");
        let mut cola = ColaEventos::default();
        let d = Decision::denegar("x");
        for _ in 0..(LOTE_REPORTE + 3) {
            cola.anotar(
                &d,
                PuntoAccesoId::nuevo(),
                None,
                SentidoAcceso::Entrada,
                None,
                ahora(),
            );
        }
        cola.borrados_biometricos.push(Uuid::now_v7());
        cola.guardar(&ruta).expect("guardar");

        let mut cargada = ColaEventos::cargar(&ruta).expect("cargar");
        assert_eq!(cargada, cola);
        let (lote, borrados) = cargada.lote();
        assert_eq!((lote.len(), borrados.len()), (LOTE_REPORTE, 1));
        cargada.confirmar(LOTE_REPORTE, 1);
        assert_eq!(cargada.eventos.len(), 3);
        assert!(cargada.borrados_biometricos.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn la_cache_se_guarda_y_pide_borrar_plantillas() {
        let tenant = Uuid::now_v7();
        let mut bio = tarjeta(tenant, Uuid::now_v7(), false);
        bio.tipo = TipoCredencial::BiometriaRef;
        bio.borrar_plantilla = true;
        let mut cache = Cache::default();
        let borrar = cache.aplicar(sincronizacion(tenant, vec![bio.clone()]));
        assert_eq!(borrar, vec![CredencialId::from_uuid(bio.id)]);

        let dir = std::env::temp_dir().join(format!("puente-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("dir");
        let ruta = dir.join("cache.json");
        cache.guardar(&ruta).expect("guardar");
        assert_eq!(Cache::cargar(&ruta).expect("cargar"), cache);
        assert_eq!(
            Cache::cargar(&dir.join("no-existe.json")).expect("vacía"),
            Cache::default()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn lineas_del_simulador() {
        let p = Uuid::now_v7();
        let l = leer_linea_simulador(&format!("{p} entrada pin 1234")).expect("línea");
        assert_eq!(
            l.lectura,
            Lectura::Credencial {
                tipo: TipoCredencial::Pin,
                valor: "1234".into()
            }
        );
        assert!(matches!(
            leer_linea_simulador(&format!("{p} salida qr DQ1.aa.bb")).map(|x| x.lectura),
            Some(Lectura::Qr(_))
        ));
        assert!(leer_linea_simulador("x entrada pin 1").is_none());
        assert!(leer_linea_simulador(&format!("{p} entrada pin")).is_none());
    }
}
