//! Consulta de la auditoría de la copropiedad (`auditoria.ver`): quién hizo qué, sobre qué
//! entidad y cuándo. Solo lectura; el detalle nunca trae datos personales ni el contenido de un
//! voto. El cursor de la paginación es el id numérico del evento, entregado como UUID opaco.

use uuid::Uuid;

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    puertos::{
        UnidadDeTrabajo,
        auditoria::{FiltroAuditoria, LecturaAuditoria, RegistroAuditado},
    },
};

/// Cursor opaco a partir del id numérico.
pub fn cursor_de(id: i64) -> Uuid {
    Uuid::from_u128(u128::try_from(id).unwrap_or(0))
}

fn id_de(cursor: Uuid) -> AppResult<i64> {
    i64::try_from(cursor.as_u128())
        .map_err(|_| AppError::Validacion("cursor de auditoría inválido".into()))
}

/// Eventos de la copropiedad, más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `auditoria.ver`; [`AppError::Validacion`] con un cursor
/// inválido o un rango de fechas invertido.
pub async fn consultar_auditoria<U>(
    unidad: &U,
    ctx: &Contexto,
    filtro: FiltroAuditoria,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<RegistroAuditado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: LecturaAuditoria,
{
    ctx.exigir(domain::Permiso::AuditoriaVer)?;
    if let (Some(d), Some(h)) = (filtro.desde, filtro.hasta)
        && h <= d
    {
        return Err(AppError::Validacion(
            "`hasta` debe ser posterior a `desde`".into(),
        ));
    }
    let antes_de = pagina.despues().map(id_de).transpose()?;
    let mut tx = unidad.iniciar(ctx).await?;
    let filas = tx
        .consultar_auditoria(&filtro, antes_de, pagina.filas_a_consultar())
        .await?;
    Ok(Pagina::desde_filas(filas, pagina, |r| cursor_de(r.id)))
}

#[cfg(test)]
mod tests {
    use domain::{CuentaId, Permiso, TenantId};

    use super::*;
    use crate::{
        auditoria::{AccionAuditoria, EventoAuditoria},
        puertos::{RegistroAuditoria, Transaccion, unidad_trabajo::memoria::UnidadEnMemoria},
    };

    const UNA: AccionAuditoria = AccionAuditoria::new("acceso.visita.anunciar");
    const OTRA: AccionAuditoria = AccionAuditoria::new("tickets.ticket.radicar");

    async fn con_eventos() -> (UnidadEnMemoria, Contexto) {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let mut tx = unidad.iniciar(&ctx).await.expect("tx");
        for (i, a) in [UNA, OTRA, UNA].into_iter().enumerate() {
            tx.registrar(EventoAuditoria::ok(a, "visita", Uuid::from_u128(i as u128)))
                .await
                .expect("registrar");
        }
        tx.confirmar().await.expect("confirmar");
        (unidad, ctx)
    }

    #[tokio::test]
    async fn filtra_por_prefijo_y_pagina_por_cursor() {
        let (unidad, ctx) = con_eventos().await;
        let filtro = FiltroAuditoria {
            accion: Some("acceso.".into()),
            ..FiltroAuditoria::default()
        };

        let p1 = consultar_auditoria(
            &unidad,
            &ctx,
            filtro.clone(),
            SolicitudPagina::new(Some(1), None).expect("página"),
        )
        .await
        .expect("consulta");
        assert_eq!(p1.datos.len(), 1);
        let p2 = consultar_auditoria(
            &unidad,
            &ctx,
            filtro,
            SolicitudPagina::new(Some(1), p1.siguiente).expect("página"),
        )
        .await
        .expect("consulta");
        assert_eq!(p2.datos.len(), 1);
        assert!(p2.datos[0].id < p1.datos[0].id, "más recientes primero");
        assert!(p2.siguiente.is_none());
    }

    #[tokio::test]
    async fn exige_el_permiso() {
        let (unidad, ctx) = con_eventos().await;
        let sin = Contexto::de_cuenta(
            ctx.tenant_id,
            CuentaId::nuevo(),
            domain::ConjuntoPermisos::vacio(),
        );

        let r = consultar_auditoria(
            &unidad,
            &sin,
            FiltroAuditoria::default(),
            SolicitudPagina::default(),
        )
        .await;

        assert!(matches!(
            r,
            Err(AppError::SinPermiso(Permiso::AuditoriaVer))
        ));
    }
}
