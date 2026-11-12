//! Cuadros de coeficientes versionados: crear un borrador (opcionalmente copiando el vigente),
//! cargar sus coeficientes, revisarlo (suma, diferencia, faltantes) y activarlo.
//!
//! Invariantes (plan 03 §5):
//! 1. Exactamente un cuadro vigente por copropiedad y su suma es 1 exacta.
//! 2. `bienes_privados.coeficiente` = coeficiente del cuadro vigente (lo copia la activación).
//! 6. No se activa un cuadro con una reunión instalada: las votaciones en curso usan el
//!    coeficiente copiado en cada participación.
//!
//! La base de datos repite estas reglas con triggers; aquí se validan primero para responder
//! con un error claro.

use std::collections::{BTreeMap, BTreeSet};

use domain::{
    ArchivoId, BienPrivadoId, CodigoBienPrivado, Coeficiente, CuadroCoeficientesId, EstadoCuadro,
    Permiso, RevisionCuadro, TipoCuadro, copropiedad::texto_opcional,
};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioBienesPrivados, RepositorioCoeficientes, Transaccion,
        UnidadDeTrabajo, copropiedades::CuadroCoeficientes,
    },
};

pub(crate) const ACCION_CREAR: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.cuadro_coeficientes.crear");
pub(crate) const ACCION_CARGAR: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.cuadro_coeficientes.cargar");
pub(crate) const ACCION_ACTIVAR: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.cuadro_coeficientes.activar");

/// Datos de un borrador nuevo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NuevoCuadro {
    /// Copiar los coeficientes del cuadro vigente como punto de partida.
    pub copiar_vigente: bool,
    /// P. ej. "Reforma aprobada en asamblea del 15/03/2027, escritura 1234 Notaría 5".
    pub descripcion: Option<String>,
    /// Escritura o acta de la reforma (opcional).
    pub soporte_archivo_id: Option<ArchivoId>,
}

/// Crea un cuadro borrador con la siguiente versión.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.coeficientes.editar`.
pub async fn crear_cuadro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoCuadro,
) -> AppResult<CuadroCoeficientes>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCoeficientes + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosCoeficientesEditar)?;
    let descripcion = texto_opcional("descripcion", nuevo.descripcion.as_deref(), 300)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let cuadro = CuadroCoeficientes {
        id: CuadroCoeficientesId::nuevo(),
        version: tx.ultima_version().await? + 1,
        tipo: TipoCuadro::Copropiedad,
        estado: EstadoCuadro::Borrador,
        descripcion,
        soporte_archivo_id: nuevo.soporte_archivo_id,
        vigente_desde: None,
        aprobado_por: None,
        creado_en: reloj.ahora(),
    };
    tx.insertar_cuadro(&cuadro).await?;
    if nuevo.copiar_vigente
        && let Some(vigente) = tx.cuadro_vigente().await?
    {
        let coeficientes = tx.coeficientes(vigente.id).await?;
        tx.reemplazar_coeficientes(cuadro.id, &coeficientes).await?;
    }
    let detalle = DetalleAuditoria::vacio().con("version", cuadro.version)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "cuadro_coeficientes", cuadro.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(cuadro)
}

/// Bien privado al que se le asigna un coeficiente: por id o por su código.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefBienPrivado {
    Id(BienPrivadoId),
    Codigo(CodigoBienPrivado),
}

/// Reemplaza los coeficientes del borrador por `filas` (la carga completa del cuadro).
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `bienes_privados.coeficientes.editar`.
/// - [`AppError::NoEncontrado`] si el cuadro o un bien privado no existen.
/// - [`AppError::Dominio`] si el cuadro no es borrador.
/// - [`AppError::Validacion`] si un bien privado se repite.
pub async fn cargar_coeficientes<U>(
    unidad: &U,
    ctx: &Contexto,
    cuadro_id: CuadroCoeficientesId,
    filas: Vec<(RefBienPrivado, Coeficiente)>,
) -> AppResult<RevisionCuadro>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCoeficientes + RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosCoeficientesEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let cuadro = tx
        .cuadro(cuadro_id)
        .await?
        .ok_or(AppError::NoEncontrado("cuadro de coeficientes"))?;
    cuadro.estado.exigir_borrador()?;
    let bienes = tx.todos_los_bienes().await?;
    let mut coeficientes = BTreeMap::new();
    for (referencia, coeficiente) in filas {
        let id = match &referencia {
            RefBienPrivado::Id(id) => bienes.iter().find(|b| b.id == *id),
            RefBienPrivado::Codigo(c) => bienes.iter().find(|b| b.codigo == *c),
        }
        .map(|b| b.id)
        .ok_or(AppError::NoEncontrado("bien privado"))?;
        if coeficientes.insert(id, coeficiente).is_some() {
            return Err(AppError::Validacion(
                "un bien privado aparece dos veces en el cuadro".into(),
            ));
        }
    }
    tx.reemplazar_coeficientes(cuadro_id, &coeficientes).await?;
    let activos = activos(&bienes);
    let revision = RevisionCuadro::revisar(&coeficientes, &activos);
    let detalle = DetalleAuditoria::vacio()
        .con("filas", coeficientes.len())?
        .con("suma", revision.suma.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CARGAR, "cuadro_coeficientes", cuadro_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(revision)
}

fn activos(bienes: &[crate::puertos::copropiedades::BienPrivado]) -> BTreeSet<BienPrivadoId> {
    bienes.iter().filter(|b| b.activo).map(|b| b.id).collect()
}

/// Cuadro con sus coeficientes y su revisión contra los bienes privados activos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleCuadro {
    pub cuadro: CuadroCoeficientes,
    pub coeficientes: BTreeMap<BienPrivadoId, Coeficiente>,
    pub revision: RevisionCuadro,
}

/// Revisa un cuadro: suma, diferencia frente a 1, faltantes y sobrantes (herramienta de ajuste
/// de redondeo; la corrección es humana).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.ver`; [`AppError::NoEncontrado`].
pub async fn validar_cuadro<U>(
    unidad: &U,
    ctx: &Contexto,
    cuadro_id: CuadroCoeficientesId,
) -> AppResult<DetalleCuadro>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCoeficientes + RepositorioBienesPrivados,
{
    ctx.exigir(Permiso::BienesPrivadosVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    detalle(&mut tx, cuadro_id).await
}

async fn detalle<T>(tx: &mut T, cuadro_id: CuadroCoeficientesId) -> AppResult<DetalleCuadro>
where
    T: RepositorioCoeficientes + RepositorioBienesPrivados,
{
    let cuadro = tx
        .cuadro(cuadro_id)
        .await?
        .ok_or(AppError::NoEncontrado("cuadro de coeficientes"))?;
    let coeficientes = tx.coeficientes(cuadro_id).await?;
    let activos = activos(&tx.todos_los_bienes().await?);
    Ok(DetalleCuadro {
        revision: RevisionCuadro::revisar(&coeficientes, &activos),
        cuadro,
        coeficientes,
    })
}

/// Cuadros de la copropiedad (más reciente primero).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.ver`.
pub async fn listar_cuadros<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<CuadroCoeficientes>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCoeficientes,
{
    ctx.exigir(Permiso::BienesPrivadosVer)?;
    unidad.iniciar(ctx).await?.cuadros().await
}

/// Activa un borrador: el vigente pasa a histórico y cada bien privado toma su coeficiente, en
/// una sola transacción. El soporte documental es opcional.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `bienes_privados.coeficientes.activar`.
/// - [`AppError::NoEncontrado`] si el cuadro no existe.
/// - [`AppError::Dominio`]: no es borrador, no suma exactamente 1
///   ([`domain::DominioError::CoeficientesNoSumanUno`]) o le faltan bienes activos.
/// - [`AppError::Conflicto`] si hay una reunión instalada.
pub async fn activar_cuadro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuadro_id: CuadroCoeficientesId,
) -> AppResult<CuadroCoeficientes>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCoeficientes + RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosCoeficientesActivar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let DetalleCuadro {
        mut cuadro,
        revision,
        ..
    } = detalle(&mut tx, cuadro_id).await?;
    cuadro.estado = cuadro.estado.transicionar(EstadoCuadro::Vigente)?;
    revision.exigir_valido()?;
    if tx.reuniones_instaladas().await? > 0 {
        return Err(AppError::Conflicto(
            "hay una reunión instalada: los coeficientes no cambian durante una asamblea".into(),
        ));
    }
    let anterior = tx.cuadro_vigente().await?.map(|c| c.id);
    let ahora = reloj.ahora();
    tx.activar(cuadro_id, anterior, ahora, ctx.actor_id).await?;
    let mut detalle = DetalleAuditoria::vacio().con("version", cuadro.version)?;
    if let Some(a) = anterior {
        detalle = detalle.con("cuadro_anterior_id", a.to_string())?;
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_ACTIVAR, "cuadro_coeficientes", cuadro_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    cuadro.vigente_desde = Some(ahora);
    cuadro.aprobado_por = ctx.actor_id;
    Ok(cuadro)
}

#[cfg(test)]
mod tests {
    use domain::{DominioError, TenantId};
    use rust_decimal::{Decimal, dec};

    use super::*;
    use crate::{
        casos_uso::bienes_privados::{crear_bien_privado, tests::nuevo_bien},
        puertos::{RelojSistema, unidad_trabajo::memoria::UnidadEnMemoria},
    };

    async fn tres_bienes(unidad: &UnidadEnMemoria, ctx: &Contexto) -> Vec<BienPrivadoId> {
        let mut ids = Vec::new();
        for n in ["101", "102", "103"] {
            ids.push(
                crear_bien_privado(unidad, ctx, nuevo_bien(n, None))
                    .await
                    .expect("crea")
                    .id,
            );
        }
        ids
    }

    fn filas(ids: &[BienPrivadoId], valores: &[Decimal]) -> Vec<(RefBienPrivado, Coeficiente)> {
        ids.iter()
            .zip(valores)
            .map(|(id, v)| {
                (
                    RefBienPrivado::Id(*id),
                    Coeficiente::new(*v).expect("válido"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn cuadro_que_no_suma_uno_se_rechaza_y_corregido_se_activa() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let ids = tres_bienes(&unidad, &ctx).await;
        let cuadro = crear_cuadro(&unidad, &RelojSistema, &ctx, NuevoCuadro::default())
            .await
            .expect("crea");
        assert_eq!(cuadro.version, 1);

        let tercio = dec!(0.33333333);
        let revision = cargar_coeficientes(&unidad, &ctx, cuadro.id, filas(&ids, &[tercio; 3]))
            .await
            .expect("carga");
        assert_eq!(revision.diferencia, dec!(0.00000001));
        assert!(matches!(
            activar_cuadro(&unidad, &RelojSistema, &ctx, cuadro.id).await,
            Err(AppError::Dominio(
                DominioError::CoeficientesNoSumanUno { .. }
            ))
        ));

        cargar_coeficientes(
            &unidad,
            &ctx,
            cuadro.id,
            filas(&ids, &[tercio, tercio, dec!(0.33333334)]),
        )
        .await
        .expect("corrige");
        let vigente = activar_cuadro(&unidad, &RelojSistema, &ctx, cuadro.id)
            .await
            .expect("activa");
        assert_eq!(vigente.estado, EstadoCuadro::Vigente);
        let datos = unidad.datos();
        assert_eq!(
            datos
                .bienes
                .values()
                .find(|b| b.id == ids[2])
                .and_then(|b| b.coeficiente),
            Some(Coeficiente::new(dec!(0.33333334)).expect("válido"))
        );
    }

    #[tokio::test]
    async fn el_vigente_pasa_a_historico_y_un_cuadro_activo_no_se_edita() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let ids = tres_bienes(&unidad, &ctx).await;
        let valores = [dec!(0.5), dec!(0.25), dec!(0.25)];
        let v1 = crear_cuadro(&unidad, &RelojSistema, &ctx, NuevoCuadro::default())
            .await
            .expect("v1");
        cargar_coeficientes(&unidad, &ctx, v1.id, filas(&ids, &valores))
            .await
            .expect("carga");
        activar_cuadro(&unidad, &RelojSistema, &ctx, v1.id)
            .await
            .expect("activa v1");
        assert!(matches!(
            cargar_coeficientes(&unidad, &ctx, v1.id, filas(&ids, &valores)).await,
            Err(AppError::Dominio(DominioError::TransicionInvalida { .. }))
        ));

        let v2 = crear_cuadro(
            &unidad,
            &RelojSistema,
            &ctx,
            NuevoCuadro {
                copiar_vigente: true,
                ..NuevoCuadro::default()
            },
        )
        .await
        .expect("v2");
        assert_eq!(v2.version, 2);
        let d = validar_cuadro(&unidad, &ctx, v2.id).await.expect("revisa");
        assert!(d.revision.es_valido(), "la copia del vigente suma 1");
        activar_cuadro(&unidad, &RelojSistema, &ctx, v2.id)
            .await
            .expect("activa v2");
        let estados: Vec<_> = listar_cuadros(&unidad, &ctx)
            .await
            .expect("lista")
            .into_iter()
            .map(|c| (c.version, c.estado))
            .collect();
        assert!(estados.contains(&(1, EstadoCuadro::Historico)));
        assert!(estados.contains(&(2, EstadoCuadro::Vigente)));
    }

    #[tokio::test]
    async fn no_se_activa_con_una_reunion_instalada() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let ids = tres_bienes(&unidad, &ctx).await;
        let c = crear_cuadro(&unidad, &RelojSistema, &ctx, NuevoCuadro::default())
            .await
            .expect("crea");
        cargar_coeficientes(
            &unidad,
            &ctx,
            c.id,
            filas(&ids, &[dec!(0.5), dec!(0.3), dec!(0.2)]),
        )
        .await
        .expect("carga");
        unidad.fijar_reuniones_instaladas(1);
        assert!(matches!(
            activar_cuadro(&unidad, &RelojSistema, &ctx, c.id).await,
            Err(AppError::Conflicto(_))
        ));
    }

    #[tokio::test]
    async fn faltantes_y_permiso_de_activar() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let ids = tres_bienes(&unidad, &ctx).await;
        let c = crear_cuadro(&unidad, &RelojSistema, &ctx, NuevoCuadro::default())
            .await
            .expect("crea");
        cargar_coeficientes(
            &unidad,
            &ctx,
            c.id,
            filas(&ids[..2], &[dec!(0.5), dec!(0.5)]),
        )
        .await
        .expect("carga");
        assert!(matches!(
            activar_cuadro(&unidad, &RelojSistema, &ctx, c.id).await,
            Err(AppError::Dominio(DominioError::CuadroIncompleto {
                faltantes: 1
            }))
        ));
        let editor = Contexto::de_cuenta(
            ctx.tenant_id,
            domain::CuentaId::nuevo(),
            [
                Permiso::BienesPrivadosCoeficientesEditar,
                Permiso::BienesPrivadosVer,
            ]
            .into_iter()
            .collect(),
        );
        assert!(matches!(
            activar_cuadro(&unidad, &RelojSistema, &editor, c.id).await,
            Err(AppError::SinPermiso(
                Permiso::BienesPrivadosCoeficientesActivar
            ))
        ));
    }
}
