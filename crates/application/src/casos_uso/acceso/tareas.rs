//! Tareas del worker de acceso (gateway):
//!
//! - [`escalar_avisos`] (cada pocos segundos): segundo aviso de visitas a los 2 min, llamada a
//!   los 5 min, vencimiento de autorizaciones, avisos de correspondencia cada 24 h y alerta a la
//!   administración al agotar los días de aviso.
//! - [`aplicar_retencion`] (diaria): borra firmas y fotos de entregas vencidas, fotos de placas
//!   y eventos más viejos que el plazo de la PH; prepara las particiones mensuales.
//!
//! Cada copropiedad se procesa en su propia transacción: un error en una no detiene las demás.

use chrono::{DateTime, Utc};
use domain::{
    EstadoCorrespondencia, EstadoVisita, NivelAviso, PasoVisita, Permiso, TenantId,
    TipoEventoVisita,
    acceso::{momento_siguiente_paso, paso_pendiente_visita},
};

use super::{
    TxAcceso, abrir,
    correspondencia::avisar_correspondencia,
    visitas::{avisar_visita, evento, pedir_llamada},
};
use crate::{
    AppError, AppResult, Contexto,
    casos_uso::avisos::{Aviso, avisar_con_permiso},
    outbox::PrioridadSalida,
    puertos::{
        AlmacenArchivos, DirectorioAcceso, Reloj, RepositorioAcceso, Transaccion, UnidadDeTrabajo,
        archivos::RepositorioArchivos,
    },
};

/// Lo que hizo una pasada de [`escalar_avisos`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenEscalamiento {
    pub copropiedades: u32,
    pub segundos_avisos: u32,
    pub llamadas: u32,
    pub vencidas: u32,
    pub avisos_correspondencia: u32,
    pub alertas_administracion: u32,
}

impl ResumenEscalamiento {
    pub const fn hubo_trabajo(&self) -> bool {
        self.segundos_avisos
            + self.llamadas
            + self.vencidas
            + self.avisos_correspondencia
            + self.alertas_administracion
            > 0
    }
}

/// Escala visitas y correspondencia pendientes de todas las copropiedades.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si no se puede consultar el directorio.
pub async fn escalar_avisos<D, U>(
    directorio: &D,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<ResumenEscalamiento>
where
    D: DirectorioAcceso,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    let mut r = ResumenEscalamiento::default();
    for tenant in directorio.copropiedades_con_acceso_pendiente(ahora).await? {
        r.copropiedades += 1;
        match escalar_en(unidad, reloj, tenant, ahora).await {
            Ok(parcial) => {
                r.segundos_avisos += parcial.segundos_avisos;
                r.llamadas += parcial.llamadas;
                r.vencidas += parcial.vencidas;
                r.avisos_correspondencia += parcial.avisos_correspondencia;
                r.alertas_administracion += parcial.alertas_administracion;
            }
            Err(AppError::ModuloNoHabilitado(_)) => {}
            Err(e) => {
                tracing::error!(tenant_id = %tenant, error = %e, "no se pudieron escalar los avisos de acceso")
            }
        }
    }
    Ok(r)
}

async fn escalar_en<U>(
    unidad: &U,
    reloj: &impl Reloj,
    tenant: TenantId,
    ahora: DateTime<Utc>,
) -> AppResult<ResumenEscalamiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ctx = Contexto::new(tenant);
    let mut tx = abrir(unidad, reloj, &ctx).await?;
    let mut r = ResumenEscalamiento::default();
    for v in tx.visitas_con_paso_pendiente(ahora).await? {
        let Some(mut v) = tx.visita_para_actualizar(v.id).await? else {
            continue;
        };
        match v.estado {
            EstadoVisita::Anunciada => {
                match paso_pendiente_visita(v.estado, v.creada_en, v.avisos_enviados, ahora) {
                    Some(PasoVisita::SegundoAviso) => {
                        avisar_visita(&mut tx, &ctx, &mut v, NivelAviso::Todos, ahora).await?;
                        v.siguiente_paso_en =
                            Some(momento_siguiente_paso(v.creada_en, v.avisos_enviados));
                        r.segundos_avisos += 1;
                    }
                    Some(PasoVisita::PedirLlamada) => {
                        pedir_llamada(&mut tx, &ctx, &mut v, ahora).await?;
                        r.llamadas += 1;
                    }
                    None => {
                        v.siguiente_paso_en =
                            Some(momento_siguiente_paso(v.creada_en, v.avisos_enviados))
                    }
                }
            }
            EstadoVisita::Autorizada if v.ventana_hasta.is_some_and(|h| h <= ahora) => {
                v.estado = v.estado.transicionar(EstadoVisita::Vencida)?;
                v.siguiente_paso_en = None;
                tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Vencida, &ctx, ahora))
                    .await?;
                r.vencidas += 1;
            }
            EstadoVisita::Autorizada => v.siguiente_paso_en = v.ventana_hasta,
            _ => v.siguiente_paso_en = None,
        }
        tx.actualizar_visita(&v).await?;
    }
    for c in tx.correspondencia_con_aviso_pendiente(ahora).await? {
        let Some(mut c) = tx.correspondencia_para_actualizar(c.id).await? else {
            continue;
        };
        if c.estado != EstadoCorrespondencia::Recibida {
            c.siguiente_aviso_en = None;
        } else if c.siguiente_aviso_en.is_some_and(|s| s <= ahora) {
            avisar_correspondencia(&mut tx, &ctx, &mut c, ahora).await?;
            r.avisos_correspondencia += 1;
        } else if c.siguiente_aviso_en.is_none() && c.alerta_administracion_en.is_none() {
            let dias = (ahora - c.recibida_en).num_days();
            let codigo = super::codigo_bien(&mut tx, c.bien_privado_id).await?;
            let aviso = Aviso {
                prioridad: PrioridadSalida::Evento,
                asunto: format!("Correspondencia sin recoger: {codigo}"),
                texto: format!(
                    "Hay correspondencia para {codigo} en portería hace {dias} día(s) y ya se agotaron los avisos \
                     automáticos. Contacte al residente o registre la devolución."
                ),
                plantilla: None,
            };
            avisar_con_permiso(&mut tx, Permiso::AccesoConfigurar, &aviso).await?;
            c.alerta_administracion_en = Some(ahora);
            r.alertas_administracion += 1;
        }
        tx.actualizar_correspondencia(&c).await?;
    }
    tx.confirmar().await?;
    Ok(r)
}

/// Lo que hizo una pasada de [`aplicar_retencion`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenRetencion {
    pub soportes_purgados: u32,
    pub fotos_purgadas: u32,
    pub eventos_borrados: u64,
}

/// Aplica los plazos de retención de cada PH (Ley 1581 y guía de videovigilancia de la SIC).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si no se puede consultar el directorio.
pub async fn aplicar_retencion<D, U, A>(
    directorio: &D,
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
) -> AppResult<ResumenRetencion>
where
    D: DirectorioAcceso,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    directorio.preparar_particiones(3).await?;
    let ahora = reloj.ahora();
    let mut r = ResumenRetencion::default();
    for tenant in directorio.copropiedades_con_control_acceso().await? {
        match retencion_en(unidad, almacen, reloj, tenant, ahora).await {
            Ok(p) => {
                r.soportes_purgados += p.soportes_purgados;
                r.fotos_purgadas += p.fotos_purgadas;
                r.eventos_borrados += p.eventos_borrados;
            }
            Err(AppError::ModuloNoHabilitado(_)) => {}
            Err(e) => {
                tracing::error!(tenant_id = %tenant, error = %e, "no se pudo aplicar la retención de acceso")
            }
        }
    }
    Ok(r)
}

async fn retencion_en<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    tenant: TenantId,
    ahora: DateTime<Utc>,
) -> AppResult<ResumenRetencion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    let ctx = Contexto::new(tenant);
    let mut tx = abrir(unidad, reloj, &ctx).await?;
    let mut r = ResumenRetencion::default();
    for s in tx.soportes_vencidos(ahora).await? {
        for id in &s.archivos {
            if let Some(a) = tx.buscar(*id).await? {
                almacen.eliminar(&a.clave).await?;
            }
        }
        tx.marcar_soportes_purgados(s.correspondencia_id, ahora)
            .await?;
        r.soportes_purgados += 1;
    }
    for (evento, archivo) in tx.fotos_eventos_vencidas(ahora).await? {
        if let Some(a) = tx.buscar(archivo).await? {
            almacen.eliminar(&a.clave).await?;
        }
        tx.marcar_foto_evento_purgada(evento, ahora).await?;
        r.fotos_purgadas += 1;
    }
    let config = tx.configuracion_acceso().await?;
    r.eventos_borrados = tx
        .purgar_eventos_acceso(config.retencion_eventos_dias)
        .await?;
    tx.confirmar().await?;
    Ok(r)
}
