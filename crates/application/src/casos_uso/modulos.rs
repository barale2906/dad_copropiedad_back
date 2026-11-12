//! Habilitación de módulos por copropiedad (venta por módulo).
//!
//! Los endpoints de cada módulo llaman a [`exigir_modulo`] antes de operar; cuando exista la
//! sesión (plan 02), un extractor lo hará por todo el router del módulo. Habilitar y terminar
//! módulos es una operación de plataforma (roles de plataforma, plan 02).

use chrono::{DateTime, Utc};
use domain::{ConjuntoPermisos, Modulo};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioModulos, Transaccion, UnidadDeTrabajo,
        modulos::PeriodoModulo,
    },
};

pub(crate) const ACCION_HABILITAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.modulo.habilitar");
pub(crate) const ACCION_TERMINAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.modulo.terminar");

/// Como [`exigir_modulo`], pero dentro de una transacción ya abierta (los casos de uso de un
/// módulo vendible lo verifican en su propia transacción).
///
/// # Errors
///
/// [`AppError::ModuloNoHabilitado`] si el módulo no está habilitado.
pub(crate) async fn exigir_modulo_en<T: RepositorioModulos>(
    tx: &mut T,
    reloj: &impl Reloj,
    modulo: Modulo,
) -> AppResult<()> {
    if tx.modulo_habilitado(modulo, reloj.ahora()).await? {
        Ok(())
    } else {
        Err(AppError::ModuloNoHabilitado(modulo))
    }
}

/// Exige que la copropiedad del contexto tenga el módulo habilitado ahora.
///
/// # Errors
///
/// [`AppError::ModuloNoHabilitado`] si no lo tiene.
pub async fn exigir_modulo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    modulo: Modulo,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioModulos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    if tx.modulo_habilitado(modulo, reloj.ahora()).await? {
        Ok(())
    } else {
        Err(AppError::ModuloNoHabilitado(modulo))
    }
}

/// Habilita el módulo desde `desde` (por defecto, ahora) hasta `hasta` (por defecto, sin fin).
///
/// # Errors
///
/// [`AppError::Validacion`] si `hasta` no es posterior a `desde`.
pub async fn habilitar_modulo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    modulo: Modulo,
    desde: Option<DateTime<Utc>>,
    hasta: Option<DateTime<Utc>>,
) -> AppResult<PeriodoModulo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioModulos + RegistroAuditoria,
{
    let periodo = PeriodoModulo {
        modulo,
        desde: desde.unwrap_or_else(|| reloj.ahora()),
        hasta,
    };
    if periodo.hasta.is_some_and(|h| h <= periodo.desde) {
        return Err(AppError::Validacion(
            "la fecha de fin debe ser posterior a la de inicio".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar_periodo(&periodo).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("modulo", modulo.as_str())?
        .con("desde", periodo.desde.to_rfc3339())?
        .con("hasta", periodo.hasta.map(|h| h.to_rfc3339()))?;
    tx.registrar(evento(ACCION_HABILITAR, ctx).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(periodo)
}

/// Termina el módulo desde ahora (los periodos abiertos se cierran; el historial se conserva).
/// Devuelve cuántos periodos cerró.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn terminar_modulo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    modulo: Modulo,
) -> AppResult<u64>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioModulos + RegistroAuditoria,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let terminados = tx.terminar_periodos(modulo, reloj.ahora()).await?;
    if terminados > 0 {
        let detalle = DetalleAuditoria::vacio().con("modulo", modulo.as_str())?;
        tx.registrar(evento(ACCION_TERMINAR, ctx).con_detalle(detalle))
            .await?;
    }
    tx.confirmar().await?;
    Ok(terminados)
}

/// Lo que la cuenta puede hacer en la copropiedad: con esto el frontend muestra u oculta
/// opciones. El backend vuelve a exigir permisos y módulos en cada operación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capacidades {
    /// Permisos efectivos de la cuenta (unión de sus roles vigentes).
    pub permisos: ConjuntoPermisos,
    /// Módulos habilitados ahora, en el orden de [`Modulo::TODOS`].
    pub modulos: Vec<Modulo>,
}

/// Permisos de la cuenta del contexto y módulos habilitados de la copropiedad.
///
/// No exige un permiso: no lee datos de negocio, solo devuelve los permisos que el contexto ya
/// trae (los de quien pregunta) y qué módulos tiene contratados su copropiedad.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn capacidades<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Capacidades>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioModulos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let habilitados = tx.modulos_habilitados(reloj.ahora()).await?;
    Ok(Capacidades {
        permisos: ctx.permisos.clone(),
        modulos: Modulo::TODOS
            .iter()
            .copied()
            .filter(|m| habilitados.contains(m))
            .collect(),
    })
}

fn evento(accion: AccionAuditoria, ctx: &Contexto) -> EventoAuditoria {
    EventoAuditoria::ok(accion, "copropiedad", ctx.tenant_id.as_uuid())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use domain::TenantId;

    use super::*;
    use crate::puertos::unidad_trabajo::memoria::UnidadEnMemoria;

    struct RelojEn(DateTime<Utc>);

    impl Reloj for RelojEn {
        fn ahora(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn dia(d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, 12, 0, 0)
            .single()
            .expect("fecha válida")
    }

    #[tokio::test]
    async fn sin_contrato_el_modulo_no_esta_habilitado() {
        let unidad = UnidadEnMemoria::default();

        let err = exigir_modulo(
            &unidad,
            &RelojEn(dia(6)),
            &Contexto::new(TenantId::nuevo()),
            Modulo::Asambleas,
        )
        .await
        .expect_err("no habilitado");

        assert!(matches!(
            err,
            AppError::ModuloNoHabilitado(Modulo::Asambleas)
        ));
    }

    #[tokio::test]
    async fn habilitado_solo_dentro_del_periodo_y_solo_ese_modulo() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        habilitar_modulo(
            &unidad,
            &RelojEn(dia(1)),
            &ctx,
            Modulo::Asambleas,
            Some(dia(5)),
            Some(dia(10)),
        )
        .await
        .expect("habilita");

        for (d, esperado) in [(4, false), (5, true), (9, true), (10, false)] {
            let res = exigir_modulo(&unidad, &RelojEn(dia(d)), &ctx, Modulo::Asambleas).await;
            assert_eq!(res.is_ok(), esperado, "día {d}");
        }
        assert!(
            exigir_modulo(&unidad, &RelojEn(dia(6)), &ctx, Modulo::Publicidad)
                .await
                .is_err()
        );
        assert_eq!(unidad.confirmados()[0].1.accion, ACCION_HABILITAR);
    }

    #[tokio::test]
    async fn terminar_cierra_el_periodo_abierto_y_queda_auditado() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        habilitar_modulo(
            &unidad,
            &RelojEn(dia(1)),
            &ctx,
            Modulo::Publicidad,
            None,
            None,
        )
        .await
        .expect("habilita");

        let terminados = terminar_modulo(&unidad, &RelojEn(dia(8)), &ctx, Modulo::Publicidad)
            .await
            .expect("termina");

        assert_eq!(terminados, 1);
        assert!(
            exigir_modulo(&unidad, &RelojEn(dia(7)), &ctx, Modulo::Publicidad)
                .await
                .is_ok()
        );
        assert!(
            exigir_modulo(&unidad, &RelojEn(dia(8)), &ctx, Modulo::Publicidad)
                .await
                .is_err()
        );
        assert_eq!(unidad.confirmados()[1].1.accion, ACCION_TERMINAR);
    }

    #[tokio::test]
    async fn periodo_invertido_se_rechaza() {
        let unidad = UnidadEnMemoria::default();

        let err = habilitar_modulo(
            &unidad,
            &RelojEn(dia(1)),
            &Contexto::new(TenantId::nuevo()),
            Modulo::Asambleas,
            Some(dia(5)),
            Some(dia(5)),
        )
        .await
        .expect_err("inválido");

        assert!(matches!(err, AppError::Validacion(_)));
    }

    #[tokio::test]
    async fn capacidades_son_los_permisos_de_la_cuenta_y_los_modulos_vigentes() {
        let unidad = UnidadEnMemoria::default();
        let tenant = TenantId::nuevo();
        let sistema = Contexto::new(tenant);
        for (modulo, desde, hasta) in [
            (Modulo::Publicidad, dia(1), None),
            (Modulo::Asambleas, dia(1), None),
            (Modulo::ControlAcceso, dia(1), Some(dia(3))),
        ] {
            habilitar_modulo(
                &unidad,
                &RelojEn(dia(1)),
                &sistema,
                modulo,
                Some(desde),
                hasta,
            )
            .await
            .expect("habilita");
        }
        let permisos: ConjuntoPermisos = [domain::Permiso::AsambleasVer].into_iter().collect();
        let cuenta = Contexto::de_cuenta(tenant, domain::CuentaId::nuevo(), permisos.clone());

        let c = capacidades(&unidad, &RelojEn(dia(6)), &cuenta)
            .await
            .expect("capacidades");

        assert_eq!(c.permisos, permisos);
        // El vencido no aparece; el orden es el de `Modulo::TODOS`, no el de alta.
        assert_eq!(c.modulos, vec![Modulo::Asambleas, Modulo::Publicidad]);
    }

    #[tokio::test]
    async fn sin_modulos_contratados_las_capacidades_no_traen_modulos() {
        let unidad = UnidadEnMemoria::default();

        let c = capacidades(&unidad, &RelojEn(dia(6)), &Contexto::new(TenantId::nuevo()))
            .await
            .expect("capacidades");

        assert!(c.modulos.is_empty());
    }
}
