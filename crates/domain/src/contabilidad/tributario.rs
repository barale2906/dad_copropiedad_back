//! Tributario y documentos electrónicos (plan 15, ADR 0020): perfil tributario de la entidad,
//! clasificación de lo que se cobra o se vende, validación del IVA de una línea, estados de los
//! documentos electrónicos, rangos de numeración y consolidación de la información exógena.
//!
//! Genérico (cualquier entidad). Ninguna tarifa, base ni formato vive aquí: llegan de los
//! catálogos de plataforma versionados. Sin perfil confirmado no hay automatismos.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::ResponsabilidadFiscal;
use crate::{DominioError, Nit};

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

fn transicion<E: Copy>(
    entidad: &'static str,
    desde: E,
    hacia: E,
    texto: fn(E) -> &'static str,
    permitida: bool,
) -> Result<E, DominioError> {
    if permitida {
        Ok(hacia)
    } else {
        Err(DominioError::TransicionInvalida {
            entidad,
            desde: texto(desde),
            hacia: texto(hacia),
        })
    }
}

/// Clasificación tributaria de lo que se cobra o se vende (§7.2 de la especificación).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClasificacionTributaria {
    /// Expensas comunes de la PH: no se facturan ni llevan IVA.
    Expensa,
    CuotaExtraordinaria,
    /// Uso de un bien común sin cobro.
    UsoSinCobro,
    /// Alquiler o servicio (explotación de zonas comunes): puede llevar IVA.
    AlquilerServicio,
    /// Venta de bienes: puede llevar IVA.
    Venta,
    Otra,
}

enum_texto!(ClasificacionTributaria {
    Expensa => "expensa",
    CuotaExtraordinaria => "cuota_extraordinaria",
    UsoSinCobro => "uso_sin_cobro",
    AlquilerServicio => "alquiler_servicio",
    Venta => "venta",
    Otra => "otra",
});

impl ClasificacionTributaria {
    /// ¿Puede llevar IVA? Solo el alquiler o servicio y la venta.
    pub fn admite_iva(self) -> bool {
        matches!(
            self,
            ClasificacionTributaria::AlquilerServicio | ClasificacionTributaria::Venta
        )
    }
}

/// Tratamiento de IVA de una tarifa del catálogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TratamientoIva {
    Gravado,
    Exento,
    Excluido,
    NoSujeto,
}

enum_texto!(TratamientoIva {
    Gravado => "gravado",
    Exento => "exento",
    Excluido => "excluido",
    NoSujeto => "no_sujeto",
});

/// Estado del perfil tributario: sin confirmar no hay automatismos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPerfilTributario {
    Borrador,
    Confirmado,
}

enum_texto!(EstadoPerfilTributario {
    Borrador => "borrador",
    Confirmado => "confirmado",
});

impl EstadoPerfilTributario {
    /// borrador → confirmado (lo confirmado no cambia: una versión nueva lo reemplaza).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        let ok = matches!(
            (self, hacia),
            (
                EstadoPerfilTributario::Borrador,
                EstadoPerfilTributario::Confirmado
            )
        );
        transicion("perfil tributario", self, hacia, Self::as_str, ok)
    }
}

/// Por dónde se transmiten los documentos electrónicos de la entidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalFacturacion {
    /// No factura electrónicamente.
    Ninguno,
    /// Proveedor tecnológico (plan 15 fase 3).
    Proveedor,
    /// Motor propio (plan 15 fase 6, pendiente).
    Propio,
}

enum_texto!(CanalFacturacion {
    Ninguno => "ninguno",
    Proveedor => "proveedor",
    Propio => "propio",
});

/// Datos del perfil tributario de una versión (los elige el contador; aquí solo se valida su
/// coherencia).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParametrosPerfil {
    pub responsabilidades: Vec<ResponsabilidadFiscal>,
    pub agente_retenedor: bool,
    pub responsable_iva: bool,
    pub gran_contribuyente: bool,
    pub autorretenedor: bool,
    pub contribuyente_renta: bool,
    pub facturador_electronico: bool,
    pub obligado_rub: bool,
    /// Códigos DANE (5 dígitos) donde declara ICA.
    pub municipios_ica: Vec<String>,
    /// Códigos CIIU (4 dígitos) de sus actividades.
    pub actividades_ica: Vec<String>,
    pub canal_facturacion: CanalFacturacion,
}

impl ParametrosPerfil {
    /// Coherencia del perfil.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] con la regla incumplida.
    pub fn validar(&self) -> Result<(), DominioError> {
        let todos_digitos = |v: &[String], n: usize| {
            v.iter()
                .all(|x| x.len() == n && x.chars().all(|c| c.is_ascii_digit()))
        };
        if !todos_digitos(&self.municipios_ica, 5) {
            return Err(invalido("municipios_ica", "códigos DANE de cinco dígitos"));
        }
        if !todos_digitos(&self.actividades_ica, 4) {
            return Err(invalido(
                "actividades_ica",
                "códigos CIIU de cuatro dígitos",
            ));
        }
        let tiene = |r: ResponsabilidadFiscal| self.responsabilidades.contains(&r);
        if self.gran_contribuyente != tiene(ResponsabilidadFiscal::GranContribuyente) {
            return Err(invalido(
                "gran_contribuyente",
                "debe coincidir con la responsabilidad O-13 del RUT",
            ));
        }
        if self.autorretenedor != tiene(ResponsabilidadFiscal::Autorretenedor) {
            return Err(invalido(
                "autorretenedor",
                "debe coincidir con la responsabilidad O-15 del RUT",
            ));
        }
        if self.responsable_iva && tiene(ResponsabilidadFiscal::NoResponsable) {
            return Err(invalido(
                "responsable_iva",
                "el RUT dice R-99-PN (no responsable de IVA)",
            ));
        }
        if self.canal_facturacion != CanalFacturacion::Ninguno && !self.facturador_electronico {
            return Err(invalido(
                "canal_facturacion",
                "solo un facturador electrónico transmite documentos",
            ));
        }
        Ok(())
    }
}

/// IVA de una línea de venta: solo en alquiler o servicio y en venta, solo si la entidad tiene un
/// perfil **confirmado** de responsable de IVA, y solo con una tarifa gravada vigente del
/// catálogo. Nunca un 19 % automático: la tarifa la elige quien factura. Una línea sin IVA
/// siempre es válida.
///
/// # Errors
///
/// [`DominioError::ReglaContable`] con el motivo.
pub fn validar_iva_linea(
    clasificacion: ClasificacionTributaria,
    tarifa: Decimal,
    perfil_confirmado: Option<&ParametrosPerfil>,
    tarifas_gravadas_vigentes: &[Decimal],
) -> Result<(), DominioError> {
    if tarifa.is_zero() {
        return Ok(());
    }
    let error = |t: &str| Err(DominioError::ReglaContable(t.to_owned()));
    if !clasificacion.admite_iva() {
        return error("solo un alquiler, un servicio o una venta llevan IVA");
    }
    match perfil_confirmado {
        None => error("confirme el perfil tributario antes de facturar con IVA"),
        Some(p) if !p.responsable_iva => {
            error("la entidad no es responsable de IVA según su perfil tributario")
        }
        Some(_) if !tarifas_gravadas_vigentes.contains(&tarifa.normalize()) => {
            error("la tarifa de IVA no está en el catálogo vigente")
        }
        Some(_) => Ok(()),
    }
}

/// Clase de documento electrónico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoDocumentoElectronico {
    Factura,
    NotaCredito,
    NotaDebito,
    /// Documento soporte en adquisiciones a no obligados a facturar.
    DocumentoSoporte,
    NotaAjusteDs,
}

enum_texto!(TipoDocumentoElectronico {
    Factura => "factura",
    NotaCredito => "nota_credito",
    NotaDebito => "nota_debito",
    DocumentoSoporte => "documento_soporte",
    NotaAjusteDs => "nota_ajuste_ds",
});

impl TipoDocumentoElectronico {
    /// Tipo de resolución de numeración que usa (las notas no usan rango DIAN).
    pub fn numeracion(self) -> Option<TipoNumeracion> {
        match self {
            TipoDocumentoElectronico::Factura => Some(TipoNumeracion::Factura),
            TipoDocumentoElectronico::DocumentoSoporte => Some(TipoNumeracion::DocumentoSoporte),
            _ => None,
        }
    }
}

/// Documentos con resolución de numeración de la DIAN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoNumeracion {
    Factura,
    DocumentoSoporte,
}

enum_texto!(TipoNumeracion {
    Factura => "factura",
    DocumentoSoporte => "documento_soporte",
});

/// Estado de un documento electrónico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoDocumentoElectronico {
    PorEnviar,
    /// Recibido por el proveedor; falta la validación de la DIAN.
    Enviado,
    Validado,
    /// La DIAN lo rechazó: se corrige con un documento nuevo o una nota (QA-015).
    Rechazado,
    /// No se pudo transmitir (caída del proveedor o de la DIAN): se reintenta.
    Contingencia,
}

enum_texto!(EstadoDocumentoElectronico {
    PorEnviar => "por_enviar",
    Enviado => "enviado",
    Validado => "validado",
    Rechazado => "rechazado",
    Contingencia => "contingencia",
});

impl EstadoDocumentoElectronico {
    /// por enviar / contingencia → enviado | validado | rechazado | contingencia;
    /// enviado → validado | rechazado. Validado y rechazado son finales.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoDocumentoElectronico::{Contingencia, Enviado, PorEnviar, Rechazado, Validado};
        let ok = matches!(
            (self, hacia),
            (
                PorEnviar | Contingencia,
                Enviado | Validado | Rechazado | Contingencia
            ) | (Enviado, Validado | Rechazado)
        );
        transicion("documento electrónico", self, hacia, Self::as_str, ok)
    }

    /// ¿Se puede transmitir (o reintentar)?
    pub fn transmisible(self) -> bool {
        matches!(
            self,
            EstadoDocumentoElectronico::PorEnviar | EstadoDocumentoElectronico::Contingencia
        )
    }
}

/// Siguiente número de un rango de la resolución de numeración.
///
/// # Errors
///
/// [`DominioError::ReglaContable`] si el rango está agotado.
pub fn siguiente_numero(ultimo: Option<u64>, desde: u64, hasta: u64) -> Result<u64, DominioError> {
    let n = ultimo.map_or(desde, |u| u.max(desde - 1) + 1);
    if n > hasta {
        return Err(DominioError::ReglaContable(
            "el rango de numeración autorizado se agotó: registre la nueva resolución".into(),
        ));
    }
    Ok(n)
}

/// ¿El rango está por agotarse? (quedan menos del 10 % o menos de 50 números).
pub fn rango_por_agotarse(usado: u64, desde: u64, hasta: u64) -> bool {
    let total = hasta - desde + 1;
    let quedan = hasta.saturating_sub(usado);
    quedan < 50 || quedan * 10 < total
}

/// Evento RADIAN sobre una factura recibida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventoRadian {
    /// Acuse de recibo de la factura.
    Acuse,
    /// Recibo del bien o la prestación del servicio.
    ReciboBien,
    /// Aceptación expresa.
    Aceptacion,
    /// Reclamo (rechazo) de la factura.
    Reclamo,
}

enum_texto!(EventoRadian {
    Acuse => "acuse",
    ReciboBien => "recibo_bien",
    Aceptacion => "aceptacion",
    Reclamo => "reclamo",
});

impl EventoRadian {
    /// Eventos que deben existir antes (acuse → recibo del bien → aceptación o reclamo).
    pub fn previos(self) -> &'static [EventoRadian] {
        match self {
            EventoRadian::Acuse => &[],
            EventoRadian::ReciboBien => &[EventoRadian::Acuse],
            EventoRadian::Aceptacion | EventoRadian::Reclamo => {
                &[EventoRadian::Acuse, EventoRadian::ReciboBien]
            }
        }
    }
}

/// Estado de una factura recibida en el buzón.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoFacturaRecibida {
    Pendiente,
    /// Ya es una factura de proveedor.
    Registrada,
    Rechazada,
}

enum_texto!(EstadoFacturaRecibida {
    Pendiente => "pendiente",
    Registrada => "registrada",
    Rechazada => "rechazada",
});

/// Criterio por el que una persona es beneficiaria final (RUB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterioBeneficiario {
    Propiedad,
    Control,
    Beneficio,
}

enum_texto!(CriterioBeneficiario {
    Propiedad => "propiedad",
    Control => "control",
    Beneficio => "beneficio",
});

/// Qué lado del movimiento de una cuenta suma a un concepto de la exógena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LadoExogena {
    Debito,
    Credito,
    Neto,
}

enum_texto!(LadoExogena {
    Debito => "debito",
    Credito => "credito",
    Neto => "neto",
});

/// NIT con que se reportan las cuantías menores.
pub const NIT_CUANTIAS_MENORES: &str = "222222222";

/// Valor de un tercero en un concepto y columna, antes de consolidar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValorExogena {
    pub concepto: String,
    pub columna: String,
    /// Documento del tercero (NIT o cédula); `None` si no lo tiene.
    pub documento: Option<String>,
    pub nombre: String,
    pub valor: Decimal,
}

/// Fila del formato: un tercero (o las cuantías menores) en un concepto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaExogena {
    pub concepto: String,
    pub documento: String,
    /// Dígito de verificación si el documento es un NIT válido.
    pub dv: Option<String>,
    pub nombre: String,
    pub columnas: BTreeMap<String, Decimal>,
}

impl FilaExogena {
    fn total(&self) -> Decimal {
        self.columnas.values().sum()
    }
}

/// Resultado de consolidar la exógena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exogena {
    pub filas: Vec<FilaExogena>,
    /// Errores de validación (tercero sin documento…); con errores el archivo se genera para
    /// revisarlo, pero se marca.
    pub errores: Vec<String>,
}

/// Consolida los valores por concepto y tercero (sin duplicados), agrega en el NIT
/// [`NIT_CUANTIAS_MENORES`] a los terceros cuyo total en el concepto no llega a la cuantía menor,
/// omite las filas en cero y valida los documentos. El orden es estable (concepto, documento):
/// la misma entrada da el mismo archivo.
pub fn consolidar_exogena(valores: &[ValorExogena], cuantia_menor: Decimal) -> Exogena {
    let mut errores = Vec::new();
    let mut por_tercero: BTreeMap<(String, String), FilaExogena> = BTreeMap::new();
    for v in valores.iter().filter(|v| !v.valor.is_zero()) {
        let documento = match v
            .documento
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            Some(d) => d.to_owned(),
            None => {
                errores.push(format!(
                    "concepto {}: el tercero «{}» no tiene documento",
                    v.concepto, v.nombre
                ));
                continue;
            }
        };
        let fila = por_tercero
            .entry((v.concepto.clone(), documento.clone()))
            .or_insert_with(|| {
                let (numero, dv) = match Nit::new(&documento) {
                    Ok(n) if documento.contains('-') => {
                        (n.numero().to_owned(), Some(n.dv().to_string()))
                    }
                    _ => (documento.clone(), None),
                };
                FilaExogena {
                    concepto: v.concepto.clone(),
                    documento: numero,
                    dv,
                    nombre: v.nombre.clone(),
                    columnas: BTreeMap::new(),
                }
            });
        *fila.columnas.entry(v.columna.clone()).or_default() += v.valor;
    }
    let mut filas: Vec<FilaExogena> = Vec::new();
    let mut menores: BTreeMap<String, FilaExogena> = BTreeMap::new();
    for fila in por_tercero.into_values() {
        if fila.total().abs() < cuantia_menor {
            let m = menores
                .entry(fila.concepto.clone())
                .or_insert_with(|| FilaExogena {
                    concepto: fila.concepto.clone(),
                    documento: NIT_CUANTIAS_MENORES.into(),
                    dv: None,
                    nombre: "CUANTIAS MENORES".into(),
                    columnas: BTreeMap::new(),
                });
            for (c, v) in fila.columnas {
                *m.columnas.entry(c).or_default() += v;
            }
        } else {
            filas.push(fila);
        }
    }
    filas.extend(menores.into_values());
    filas.retain(|f| !f.total().is_zero());
    filas.sort_by(|a, b| (&a.concepto, &a.documento).cmp(&(&b.concepto, &b.documento)));
    Exogena { filas, errores }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn perfil() -> ParametrosPerfil {
        ParametrosPerfil {
            responsabilidades: vec![],
            agente_retenedor: true,
            responsable_iva: true,
            gran_contribuyente: false,
            autorretenedor: false,
            contribuyente_renta: true,
            facturador_electronico: true,
            obligado_rub: false,
            municipios_ica: vec!["11001".into()],
            actividades_ica: vec!["6820".into()],
            canal_facturacion: CanalFacturacion::Proveedor,
        }
    }

    #[test]
    fn el_perfil_debe_ser_coherente_con_el_rut() {
        assert!(perfil().validar().is_ok());
        let mut p = perfil();
        p.gran_contribuyente = true;
        assert!(p.validar().is_err());
        p.responsabilidades = vec![ResponsabilidadFiscal::GranContribuyente];
        assert!(p.validar().is_ok());
        let mut p = perfil();
        p.responsabilidades = vec![ResponsabilidadFiscal::NoResponsable];
        assert!(p.validar().is_err());
        let mut p = perfil();
        p.facturador_electronico = false;
        assert!(p.validar().is_err());
        let mut p = perfil();
        p.municipios_ica = vec!["110".into()];
        assert!(p.validar().is_err());
    }

    #[test]
    fn el_iva_solo_va_en_alquiler_o_venta_con_perfil_confirmado_y_tarifa_del_catalogo() {
        let p = perfil();
        let vigentes = [dec!(19), dec!(5)];
        assert!(validar_iva_linea(ClasificacionTributaria::Expensa, dec!(0), None, &[]).is_ok());
        assert!(
            validar_iva_linea(
                ClasificacionTributaria::Venta,
                dec!(19.00),
                Some(&p),
                &vigentes
            )
            .is_ok()
        );
        assert!(
            validar_iva_linea(
                ClasificacionTributaria::Expensa,
                dec!(19),
                Some(&p),
                &vigentes
            )
            .is_err()
        );
        assert!(
            validar_iva_linea(ClasificacionTributaria::Venta, dec!(19), None, &vigentes).is_err(),
            "perfil en borrador bloquea automatismos"
        );
        assert!(
            validar_iva_linea(
                ClasificacionTributaria::Venta,
                dec!(16),
                Some(&p),
                &vigentes
            )
            .is_err()
        );
        let mut no_responsable = perfil();
        no_responsable.responsable_iva = false;
        assert!(
            validar_iva_linea(
                ClasificacionTributaria::AlquilerServicio,
                dec!(19),
                Some(&no_responsable),
                &vigentes
            )
            .is_err()
        );
    }

    #[test]
    fn el_rango_de_numeracion_se_agota() {
        assert_eq!(siguiente_numero(None, 1, 10).expect("primero"), 1);
        assert_eq!(siguiente_numero(Some(4), 1, 10).expect("siguiente"), 5);
        assert!(siguiente_numero(Some(10), 1, 10).is_err());
        assert!(rango_por_agotarse(9_960, 1, 10_000));
        assert!(!rango_por_agotarse(100, 1, 10_000));
    }

    #[test]
    fn estados_del_documento_electronico() {
        use EstadoDocumentoElectronico::*;
        assert!(PorEnviar.transicionar(Validado).is_ok());
        assert!(Contingencia.transicionar(Enviado).is_ok());
        assert!(Rechazado.transicionar(Validado).is_err(), "QA-015");
        assert!(Validado.transicionar(Rechazado).is_err());
        assert!(Contingencia.transmisible());
        assert!(!Enviado.transmisible());
    }

    #[test]
    fn radian_exige_los_eventos_previos() {
        assert!(EventoRadian::Acuse.previos().is_empty());
        assert_eq!(
            EventoRadian::Reclamo.previos(),
            &[EventoRadian::Acuse, EventoRadian::ReciboBien]
        );
    }

    #[test]
    fn la_exogena_agrega_cuantias_menores_y_es_reproducible() {
        let v = |concepto: &str, documento: Option<&str>, valor: Decimal| ValorExogena {
            concepto: concepto.into(),
            columna: "pago_deducible".into(),
            documento: documento.map(str::to_owned),
            nombre: "Tercero".into(),
            valor,
        };
        let valores = vec![
            v("5002", Some("900123456-8"), dec!(3000000)),
            v("5002", Some("900123456-8"), dec!(500000)),
            v("5002", Some("79123456"), dec!(40000)),
            v("5002", Some("52123456"), dec!(30000)),
            v("5016", None, dec!(10)),
        ];

        let e = consolidar_exogena(&valores, dec!(100000));
        let otra = consolidar_exogena(&valores, dec!(100000));

        assert_eq!(e, otra);
        assert_eq!(e.filas.len(), 2);
        assert_eq!(e.filas[0].documento, NIT_CUANTIAS_MENORES);
        assert_eq!(e.filas[0].columnas["pago_deducible"], dec!(70000));
        assert_eq!(e.filas[1].documento, "900123456");
        assert_eq!(e.filas[1].dv.as_deref(), Some("8"));
        assert_eq!(e.filas[1].columnas["pago_deducible"], dec!(3500000));
        assert_eq!(e.errores.len(), 1);
    }
}
