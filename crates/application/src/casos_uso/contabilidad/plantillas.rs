//! Catálogo PUC y plantillas contables (principio P1): la plataforma publica plantillas
//! estándar (la de PH la arma su contador) y cada firma contable guarda las suyas para sus
//! clientes. Una entidad aplica cualquiera de ellas con
//! [`super::configuracion::aplicar_plantilla`] y la sigue editando por su cuenta.

use std::collections::BTreeSet;

use domain::{
    Permiso, PlantillaContableId,
    contabilidad::{NivelCuenta, PlantillaContable},
};

use super::{TxContable, configuracion::exportar_plantilla, es_tipo_empresa};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        contabilidad::{
            AmbitoPlantilla, CatalogoContablePlataforma, CuentaPuc, EstadoPlantilla,
            PlantillaGuardada, PlantillaResumen,
        },
    },
};

const ACCION_PUC: AccionAuditoria = AccionAuditoria::new("plataforma.puc.cargar");
const ACCION_GUARDAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.plantilla_contable.guardar");
const ACCION_ESTADO: AccionAuditoria = AccionAuditoria::new("plataforma.plantilla_contable.estado");

/// Carga (inserta o actualiza) cuentas del catálogo PUC. Solo clases, grupos, cuentas y
/// subcuentas (1, 2, 4 y 6 dígitos): los auxiliares son de cada entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si
/// hay auxiliares, nombres vacíos o códigos repetidos.
pub async fn cargar_puc<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    cuentas: Vec<CuentaPuc>,
) -> AppResult<u64>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let mut vistos = BTreeSet::new();
    for c in &cuentas {
        if c.codigo.nivel() == NivelCuenta::Auxiliar
            || c.nombre.trim().is_empty()
            || c.nombre.chars().count() > 200
            || !vistos.insert(c.codigo.as_str())
        {
            return Err(AppError::Validacion(format!(
                "la cuenta {} es un auxiliar, no tiene nombre o está repetida",
                c.codigo
            )));
        }
    }
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let n = tx.cargar_puc(&cuentas).await?;
    let detalle = DetalleAuditoria::vacio().con("cuentas", n)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PUC, "catalogo_puc", uuid::Uuid::nil()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(n)
}

/// Exige poder gestionar las plantillas del ámbito: el permiso de plataforma para las estándar;
/// ser dueño o gestor de la firma para las suyas.
async fn exigir_gestion<T: CatalogoContablePlataforma>(
    tx: &mut T,
    ctx: &ContextoPlataforma,
    ambito: AmbitoPlantilla,
) -> AppResult<()> {
    match ambito {
        AmbitoPlantilla::Plataforma => ctx.exigir(Permiso::PlataformaPlantillasContablesPublicar),
        AmbitoPlantilla::Organizacion(org) => {
            let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
            if tx.gestiona_organizacion(org, cuenta).await? {
                Ok(())
            } else {
                Err(AppError::NoEncontrado("organizacion"))
            }
        }
    }
}

/// Plantillas del ámbito (todas sus versiones y estados).
///
/// # Errors
///
/// [`AppError::SinPermiso`] o [`AppError::NoEncontrado`] si no puede gestionar el ámbito.
pub async fn plantillas<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    ambito: AmbitoPlantilla,
) -> AppResult<Vec<PlantillaResumen>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma,
{
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    exigir_gestion(&mut tx, ctx, ambito).await?;
    tx.plantillas(ambito).await
}

/// Plantilla con su contenido (de un ámbito que la cuenta gestiona).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o no la puede gestionar.
pub async fn plantilla<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    id: PlantillaContableId,
) -> AppResult<PlantillaGuardada>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma,
{
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let p = tx
        .plantilla(id)
        .await?
        .ok_or(AppError::NoEncontrado("plantilla"))?;
    exigir_gestion(&mut tx, ctx, p.ambito).await?;
    Ok(p)
}

async fn guardar_nueva<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    ambito: AmbitoPlantilla,
    nombre: &str,
    tipo_empresa: &str,
    contenido: &PlantillaContable,
) -> AppResult<PlantillaGuardada>
where
    T: CatalogoContablePlataforma + RegistroAuditoria,
{
    let nombre = super::texto("nombre", nombre, 120)?;
    if !es_tipo_empresa(tipo_empresa) {
        return Err(AppError::Validacion(
            "el tipo de empresa usa minúsculas, dígitos y `_` (p. ej. `propiedad_horizontal`)"
                .into(),
        ));
    }
    contenido.validar(&BTreeSet::new())?;
    let p = PlantillaGuardada {
        id: PlantillaContableId::nuevo(),
        ambito,
        version: tx.ultima_version_plantilla(ambito, &nombre).await? + 1,
        nombre,
        tipo_empresa: tipo_empresa.to_owned(),
        estado: EstadoPlantilla::Borrador,
        contenido: serde_json::to_value(contenido).map_err(AppError::infra)?,
        autor: ctx.actor_id,
        creada_en: reloj.ahora(),
        publicada_en: None,
    };
    tx.insertar_plantilla(&p).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("nombre", p.nombre.as_str())?
        .con("version", p.version)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_GUARDAR, "plantilla_contable", p.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(p)
}

/// Crea una plantilla en borrador (versión siguiente si el nombre ya existe en el ámbito).
///
/// # Errors
///
/// [`AppError::SinPermiso`]/[`AppError::NoEncontrado`] si no gestiona el ámbito;
/// [`AppError::Validacion`] o [`domain::DominioError::ReglaContable`] si el contenido no es
/// coherente.
pub async fn crear_plantilla<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    ambito: AmbitoPlantilla,
    nombre: &str,
    tipo_empresa: &str,
    contenido: PlantillaContable,
) -> AppResult<PlantillaGuardada>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma + RegistroAuditoria,
{
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    exigir_gestion(&mut tx, ctx, ambito).await?;
    let p = guardar_nueva(
        &mut tx,
        reloj,
        ctx,
        ambito,
        nombre,
        tipo_empresa,
        &contenido,
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Reemplaza el contenido de una plantilla en borrador.
///
/// # Errors
///
/// [`AppError::Conflicto`] si ya no es borrador (se crea otra versión).
pub async fn actualizar_plantilla<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    id: PlantillaContableId,
    contenido: PlantillaContable,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma + RegistroAuditoria,
{
    contenido.validar(&BTreeSet::new())?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let p = tx
        .plantilla(id)
        .await?
        .ok_or(AppError::NoEncontrado("plantilla"))?;
    exigir_gestion(&mut tx, ctx, p.ambito).await?;
    if p.estado != EstadoPlantilla::Borrador {
        return Err(AppError::Conflicto(
            "una plantilla publicada no se edita: cree una versión nueva".into(),
        ));
    }
    tx.actualizar_contenido_plantilla(
        id,
        &serde_json::to_value(&contenido).map_err(AppError::infra)?,
    )
    .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_GUARDAR,
        "plantilla_contable",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Publica (borrador → publicada) o retira (publicada → retirada) una plantilla.
///
/// # Errors
///
/// [`AppError::Conflicto`] si la transición no corresponde.
pub async fn cambiar_estado_plantilla<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: PlantillaContableId,
    hacia: EstadoPlantilla,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma + RegistroAuditoria,
{
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let p = tx
        .plantilla(id)
        .await?
        .ok_or(AppError::NoEncontrado("plantilla"))?;
    exigir_gestion(&mut tx, ctx, p.ambito).await?;
    let valida = matches!(
        (p.estado, hacia),
        (EstadoPlantilla::Borrador, EstadoPlantilla::Publicada)
            | (EstadoPlantilla::Publicada, EstadoPlantilla::Retirada)
    );
    if !valida {
        return Err(AppError::Conflicto(format!(
            "la plantilla está {} y no puede pasar a {}",
            p.estado.as_str(),
            hacia.as_str()
        )));
    }
    tx.cambiar_estado_plantilla(id, hacia, reloj.ahora())
        .await?;
    let detalle = DetalleAuditoria::vacio().con("estado", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO, "plantilla_contable", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Dónde y con qué nombre guardar una plantilla.
#[derive(Debug, Clone)]
pub struct DestinoPlantilla {
    pub ambito: AmbitoPlantilla,
    pub nombre: String,
    /// Tipo de empresa (`propiedad_horizontal`, `gimnasio`…).
    pub tipo_empresa: String,
}

/// Guarda la configuración contable de la entidad como plantilla en borrador del destino (su
/// firma contable o, con el permiso de plataforma, el estándar). Así el contador reutiliza lo
/// que armó para otros clientes del mismo tipo.
///
/// # Errors
///
/// Los de [`exportar_plantilla`] y los de gestionar el destino.
pub async fn guardar_como_plantilla<U, UP>(
    unidad: &U,
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &Contexto,
    ctx_plataforma: &ContextoPlataforma,
    destino: DestinoPlantilla,
) -> AppResult<PlantillaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoContablePlataforma + RegistroAuditoria,
{
    let contenido = exportar_plantilla(unidad, reloj, ctx).await?;
    let mut tx = plataforma.iniciar_plataforma(ctx_plataforma).await?;
    exigir_gestion(&mut tx, ctx_plataforma, destino.ambito).await?;
    let p = guardar_nueva(
        &mut tx,
        reloj,
        ctx_plataforma,
        destino.ambito,
        &destino.nombre,
        &destino.tipo_empresa,
        &contenido,
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}
