//! Casos de uso de protección de datos personales (Ley 1581 de 2012).
//!
//! - [`publicar_politica`]: nueva versión de la política de tratamiento de la copropiedad.
//! - [`registrar_autorizacion`] / [`revocar_autorizacion`]: autorizaciones por finalidad.
//! - [`esta_autorizado`]: lo consultan los módulos antes de tratar datos para una finalidad
//!   (p. ej. publicidad o comunicaciones).
//! - [`radicar_solicitud`]: consulta o reclamo del titular con su vencimiento legal en días
//!   hábiles de Colombia.
//!
//! La auditoría guarda ids, finalidades y fechas, nunca los datos personales.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, AutorizacionId, FinalidadTratamiento, MedioAutorizacion, PersonaId,
    PoliticaTratamientoId, SolicitudTitularId, TipoSolicitudTitular,
    proteccion_datos::fecha_en_colombia,
};
use serde_json::Value;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioProteccionDatos, Transaccion, UnidadDeTrabajo,
        proteccion_datos::{Autorizacion, PoliticaTratamiento, SolicitudTitular},
    },
};

pub const ACCION_PUBLICAR_POLITICA: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.politica.publicar");
pub const ACCION_OTORGAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.autorizacion.otorgar");
pub const ACCION_REVOCAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.autorizacion.revocar");
pub const ACCION_RADICAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.solicitud.radicar");

/// Contenido de una política nueva: texto, archivo (PDF) o ambos.
#[derive(Debug, Clone)]
pub struct NuevaPolitica {
    pub texto: Option<String>,
    pub archivo_id: Option<ArchivoId>,
    /// `None` = desde ahora.
    pub vigente_desde: Option<DateTime<Utc>>,
}

/// Publica una versión nueva de la política (la siguiente a la última).
///
/// # Errors
///
/// [`AppError::Validacion`] si no trae ni texto ni archivo, o si `vigente_desde` es pasado.
pub async fn publicar_politica<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaPolitica,
) -> AppResult<PoliticaTratamiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos + RegistroAuditoria,
{
    let texto = nueva.texto.filter(|t| !t.trim().is_empty());
    if texto.is_none() && nueva.archivo_id.is_none() {
        return Err(AppError::Validacion(
            "la política necesita un texto o un archivo".into(),
        ));
    }
    let ahora = reloj.ahora();
    let vigente_desde = nueva.vigente_desde.unwrap_or(ahora);
    if vigente_desde < ahora {
        // Las autorizaciones ya otorgadas se dieron bajo otra versión: no se reescribe el pasado.
        return Err(AppError::Validacion(
            "la vigencia de una política no puede empezar en el pasado".into(),
        ));
    }

    let mut tx = unidad.iniciar(ctx).await?;
    let politica = PoliticaTratamiento {
        id: PoliticaTratamientoId::nuevo(),
        version: tx.ultima_version_politica().await? + 1,
        texto,
        archivo_id: nueva.archivo_id,
        vigente_desde,
    };
    tx.insertar_politica(&politica).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("version", politica.version)?
        .con("vigente_desde", politica.vigente_desde.to_rfc3339())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_PUBLICAR_POLITICA,
            "politica_tratamiento",
            politica.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(politica)
}

/// Autorización que llega del titular.
#[derive(Debug, Clone)]
pub struct AutorizacionOtorgada {
    pub persona_id: PersonaId,
    pub finalidades: BTreeSet<FinalidadTratamiento>,
    pub medio: MedioAutorizacion,
    pub evidencia_archivo_id: Option<ArchivoId>,
}

/// Registra la autorización del titular para cada finalidad, bajo la política vigente. Las
/// finalidades que ya estaban autorizadas no se duplican. Devuelve las que quedaron nuevas.
///
/// # Errors
///
/// [`AppError::Validacion`] si no hay finalidades o la copropiedad no tiene política vigente
/// (sin política no hay autorización informada, art. 12).
pub async fn registrar_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    otorgada: AutorizacionOtorgada,
) -> AppResult<Vec<FinalidadTratamiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos + RegistroAuditoria,
{
    if otorgada.finalidades.is_empty() {
        return Err(AppError::Validacion(
            "indique al menos una finalidad".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let politica = tx.politica_vigente(ahora).await?.ok_or_else(|| {
        AppError::Validacion("la copropiedad no tiene una política de tratamiento vigente".into())
    })?;

    let mut nuevas = Vec::new();
    for finalidad in otorgada.finalidades {
        let autorizacion = Autorizacion {
            id: AutorizacionId::nuevo(),
            persona_id: otorgada.persona_id,
            politica_id: politica.id,
            finalidad,
            medio: otorgada.medio,
            evidencia_archivo_id: otorgada.evidencia_archivo_id,
            otorgada_en: ahora,
        };
        if tx.insertar_autorizacion(&autorizacion).await? {
            nuevas.push(finalidad);
        }
    }
    if !nuevas.is_empty() {
        let detalle = DetalleAuditoria::vacio()
            .con("finalidades", nombres(&nuevas))?
            .con("medio", otorgada.medio.as_str())?
            .con("politica_version", politica.version)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_OTORGAR, "persona", otorgada.persona_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(nuevas)
}

/// Revoca la autorización del titular para esas finalidades (art. 8, lit. e). Devuelve cuántas
/// autorizaciones vigentes se revocaron.
///
/// # Errors
///
/// [`AppError::Validacion`] si no se indica ninguna finalidad.
pub async fn revocar_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona_id: PersonaId,
    finalidades: &[FinalidadTratamiento],
) -> AppResult<u64>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos + RegistroAuditoria,
{
    if finalidades.is_empty() {
        return Err(AppError::Validacion(
            "indique al menos una finalidad".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    let revocadas = tx
        .revocar_autorizaciones(persona_id, finalidades, reloj.ahora())
        .await?;
    if revocadas > 0 {
        let detalle = DetalleAuditoria::vacio().con("finalidades", nombres(finalidades))?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_REVOCAR, "persona", persona_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(revocadas)
}

/// ¿La persona tiene autorización vigente para la finalidad?
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn esta_autorizado<U>(
    unidad: &U,
    ctx: &Contexto,
    persona_id: PersonaId,
    finalidad: FinalidadTratamiento,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    Ok(tx
        .finalidades_autorizadas(persona_id)
        .await?
        .contains(&finalidad))
}

/// Radica una consulta o un reclamo del titular y calcula su vencimiento legal (10 o 15 días
/// hábiles desde el día de radicación en Colombia).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos o el vencimiento se sale del
/// calendario.
pub async fn radicar_solicitud<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona_id: PersonaId,
    tipo: TipoSolicitudTitular,
) -> AppResult<SolicitudTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos + RegistroAuditoria,
{
    let radicada_en = reloj.ahora();
    let vence_el = tipo
        .vencimiento(fecha_en_colombia(radicada_en))
        .ok_or_else(|| AppError::infra(std::io::Error::other("vencimiento fuera de rango")))?;
    let solicitud = SolicitudTitular {
        id: SolicitudTitularId::nuevo(),
        persona_id,
        tipo,
        radicada_en,
        vence_el,
    };

    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar_solicitud(&solicitud).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", tipo.as_str())?
        .con("vence_el", vence_el.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RADICAR, "solicitud_titular", solicitud.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(solicitud)
}

fn nombres(finalidades: &[FinalidadTratamiento]) -> Value {
    Value::Array(
        finalidades
            .iter()
            .map(|f| Value::from(f.as_str()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use domain::TenantId;

    use super::*;
    use crate::puertos::unidad_trabajo::memoria::UnidadEnMemoria;

    /// Viernes 30 de octubre de 2026, 3 p. m. en Colombia.
    struct RelojFijo;

    impl Reloj for RelojFijo {
        fn ahora(&self) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 10, 30, 20, 0, 0)
                .single()
                .expect("fecha válida")
        }
    }

    fn politica_de_texto() -> NuevaPolitica {
        NuevaPolitica {
            texto: Some("Política de tratamiento de datos del Conjunto Alameda".into()),
            archivo_id: None,
            vigente_desde: None,
        }
    }

    fn otorgada(persona: PersonaId, f: &[FinalidadTratamiento]) -> AutorizacionOtorgada {
        AutorizacionOtorgada {
            persona_id: persona,
            finalidades: f.iter().copied().collect(),
            medio: MedioAutorizacion::WhatsApp,
            evidencia_archivo_id: None,
        }
    }

    async fn con_politica() -> (UnidadEnMemoria, Contexto) {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        publicar_politica(&unidad, &RelojFijo, &ctx, politica_de_texto())
            .await
            .expect("publica");
        (unidad, ctx)
    }

    #[tokio::test]
    async fn cada_politica_publicada_es_una_version_nueva() {
        let (unidad, ctx) = con_politica().await;

        let segunda = publicar_politica(&unidad, &RelojFijo, &ctx, politica_de_texto())
            .await
            .expect("publica");

        assert_eq!(segunda.version, 2);
        assert_eq!(unidad.datos().politicas.len(), 2);
        let acciones: Vec<_> = unidad.confirmados().iter().map(|e| e.1.accion).collect();
        assert_eq!(acciones, vec![ACCION_PUBLICAR_POLITICA; 2]);
    }

    #[tokio::test]
    async fn politica_vacia_o_con_vigencia_pasada_se_rechaza() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let vacia = NuevaPolitica {
            texto: Some("   ".into()),
            archivo_id: None,
            vigente_desde: None,
        };
        let pasada = NuevaPolitica {
            vigente_desde: Some(RelojFijo.ahora() - chrono::Duration::days(1)),
            ..politica_de_texto()
        };

        for nueva in [vacia, pasada] {
            let err = publicar_politica(&unidad, &RelojFijo, &ctx, nueva)
                .await
                .expect_err("se rechaza");
            assert!(matches!(err, AppError::Validacion(_)));
        }
        assert!(unidad.datos().politicas.is_empty());
    }

    #[tokio::test]
    async fn sin_politica_vigente_no_se_puede_autorizar() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        // Una política que empieza a regir mañana todavía no sirve.
        publicar_politica(
            &unidad,
            &RelojFijo,
            &ctx,
            NuevaPolitica {
                vigente_desde: Some(RelojFijo.ahora() + chrono::Duration::days(1)),
                ..politica_de_texto()
            },
        )
        .await
        .expect("publica");

        let err = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(PersonaId::nuevo(), &[FinalidadTratamiento::Comunicaciones]),
        )
        .await
        .expect_err("sin política vigente");

        assert!(matches!(err, AppError::Validacion(_)));
    }

    #[tokio::test]
    async fn autorizacion_por_finalidad_sin_duplicados_y_auditada_sin_datos_personales() {
        let (unidad, ctx) = con_politica().await;
        let persona = PersonaId::nuevo();
        use FinalidadTratamiento::*;

        let primeras = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Administracion, Comunicaciones]),
        )
        .await
        .expect("autoriza");
        let repetidas = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Comunicaciones, Publicidad]),
        )
        .await
        .expect("autoriza");

        assert_eq!(primeras, vec![Administracion, Comunicaciones]);
        assert_eq!(repetidas, vec![Publicidad], "comunicaciones ya estaba");
        assert_eq!(unidad.datos().autorizaciones.len(), 3);
        assert!(
            esta_autorizado(&unidad, &ctx, persona, Publicidad)
                .await
                .expect("consulta")
        );
        assert!(
            !esta_autorizado(&unidad, &ctx, persona, ControlAcceso)
                .await
                .expect("consulta")
        );
        let ultimo = unidad.confirmados().pop().expect("auditado").1;
        assert_eq!(ultimo.accion, ACCION_OTORGAR);
        assert_eq!(
            ultimo.detalle.a_json()["finalidades"],
            serde_json::json!(["publicidad"])
        );
    }

    #[tokio::test]
    async fn revocar_quita_solo_las_finalidades_indicadas() {
        let (unidad, ctx) = con_politica().await;
        let persona = PersonaId::nuevo();
        use FinalidadTratamiento::*;
        registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Comunicaciones, Publicidad]),
        )
        .await
        .expect("autoriza");

        let revocadas = revocar_autorizacion(&unidad, &RelojFijo, &ctx, persona, &[Publicidad])
            .await
            .expect("revoca");
        let otra_vez = revocar_autorizacion(&unidad, &RelojFijo, &ctx, persona, &[Publicidad])
            .await
            .expect("revoca");

        assert_eq!((revocadas, otra_vez), (1, 0));
        assert!(
            !esta_autorizado(&unidad, &ctx, persona, Publicidad)
                .await
                .expect("consulta")
        );
        assert!(
            esta_autorizado(&unidad, &ctx, persona, Comunicaciones)
                .await
                .expect("consulta")
        );
        // Se vuelve a autorizar después de revocar: queda una autorización nueva.
        let nuevas =
            registrar_autorizacion(&unidad, &RelojFijo, &ctx, otorgada(persona, &[Publicidad]))
                .await
                .expect("autoriza");
        assert_eq!(nuevas, vec![Publicidad]);
    }

    #[tokio::test]
    async fn solicitud_vence_en_dias_habiles_de_colombia() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let persona = PersonaId::nuevo();

        let consulta = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &ctx,
            persona,
            TipoSolicitudTitular::Consulta,
        )
        .await
        .expect("radica");
        let supresion = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &ctx,
            persona,
            TipoSolicitudTitular::Supresion,
        )
        .await
        .expect("radica");

        assert_eq!(
            consulta.vence_el,
            NaiveDate::from_ymd_opt(2026, 11, 17).expect("fecha")
        );
        assert_eq!(
            supresion.vence_el,
            NaiveDate::from_ymd_opt(2026, 11, 24).expect("fecha")
        );
        assert_eq!(unidad.datos().solicitudes.len(), 2);
        assert_eq!(unidad.confirmados()[0].1.accion, ACCION_RADICAR);
    }
}
