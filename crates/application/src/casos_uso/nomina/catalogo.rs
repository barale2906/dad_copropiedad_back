//! Catálogo laboral de la plataforma (plan 16 fase 1): parámetros versionados (salario mínimo,
//! recargos, aportes, prestaciones…) y administradoras de la seguridad social. Lo mantiene el
//! equipo de la plataforma con su especialista laboral (`plataforma.normativa.gestionar`); las
//! entidades con el módulo de nómina lo leen vigente en una fecha.

use chrono::NaiveDate;
use domain::{
    AdministradoraId, Nit, ParametroLaboralId, Permiso,
    nomina::{
        CatalogoLaboral, ParametroLaboral, TipoAdministradora, VersionParametro,
        liquidacion::{TablaLaboral, TablaTramos, Tramo},
        validar_codigo_administradora,
    },
};
use rust_decimal::Decimal;

use super::{TxNomina, abrir_nomina};
use crate::casos_uso::contabilidad::texto;
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        nomina::{
            Administradora, RepositorioCatalogoLaboral, VersionParametroLaboral,
            VersionTablaLaboral,
        },
    },
};

const ACCION_PARAMETRO: AccionAuditoria =
    AccionAuditoria::new("plataforma.parametro_laboral.crear");
const ACCION_ADMINISTRADORA: AccionAuditoria =
    AccionAuditoria::new("plataforma.administradora.crear");
const ACCION_ADMINISTRADORA_EDITAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.administradora.editar");
const ACCION_TABLA: AccionAuditoria = AccionAuditoria::new("plataforma.tabla_laboral.crear");

/// Versión nueva de un parámetro.
#[derive(Debug, Clone)]
pub struct NuevoParametro {
    pub parametro: ParametroLaboral,
    pub valor: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Agrega una versión de un parámetro laboral (solo inserción: vale la de mayor
/// `vigente_desde` hasta cada fecha; un error se corrige con otra versión).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`;
/// [`domain::DominioError::ReglaLaboral`] si el valor no cabe en la unidad del parámetro;
/// [`AppError::Conflicto`] si ya hay una versión con esa fecha.
pub async fn crear_parametro<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nuevo: NuevoParametro,
) -> AppResult<VersionParametroLaboral>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCatalogoLaboral + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let version = VersionParametroLaboral {
        id: ParametroLaboralId::nuevo(),
        parametro: nuevo.parametro,
        valor: nuevo.parametro.validar_valor(nuevo.valor)?,
        fuente: texto("fuente", &nuevo.fuente, 300)?,
        vigente_desde: nuevo.vigente_desde,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_parametro(&version).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("parametro", version.parametro.as_str())?
        .con("valor", version.valor.to_string())?
        .con("vigente_desde", version.vigente_desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PARAMETRO, "parametro_laboral", version.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(version)
}

/// Administradora nueva de la seguridad social.
#[derive(Debug, Clone)]
pub struct NuevaAdministradora {
    pub tipo: TipoAdministradora,
    /// Código en la PILA.
    pub codigo: String,
    pub nit: Nit,
    pub nombre: String,
}

/// Agrega una administradora (EPS, AFP, cesantías, caja o ARL) al catálogo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Dominio`] o
/// [`AppError::Validacion`] si un dato no cumple; [`AppError::Conflicto`] si ya existe una del
/// mismo tipo con ese código.
pub async fn crear_administradora<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nueva: NuevaAdministradora,
) -> AppResult<Administradora>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCatalogoLaboral + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let administradora = Administradora {
        id: AdministradoraId::nuevo(),
        tipo: nueva.tipo,
        codigo: validar_codigo_administradora(&nueva.codigo)?,
        nit: nueva.nit,
        nombre: texto("nombre", &nueva.nombre, 200)?,
        activa: true,
        creada_en: reloj.ahora(),
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_administradora(&administradora).await?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_ADMINISTRADORA,
            "administradora",
            administradora.id.as_uuid(),
        )
        .con_detalle(auditar(&administradora)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(administradora)
}

fn auditar(a: &Administradora) -> AppResult<DetalleAuditoria> {
    DetalleAuditoria::vacio()
        .con("tipo", a.tipo.as_str())?
        .con("codigo", a.codigo.as_str())?
        .con("activa", a.activa)
}

/// Cambia el nombre y el estado de una administradora. Una inactiva no se usa en contratos ni
/// versiones nuevas; las existentes no cambian.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el nombre no es válido.
pub async fn actualizar_administradora<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    id: AdministradoraId,
    nombre: &str,
    activa: bool,
) -> AppResult<Administradora>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCatalogoLaboral + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let nombre = texto("nombre", nombre, 200)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let mut a = tx
        .administradora(id)
        .await?
        .ok_or(AppError::NoEncontrado("administradora"))?;
    a.nombre = nombre;
    a.activa = activa;
    tx.actualizar_administradora(&a).await?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_ADMINISTRADORA_EDITAR,
            "administradora",
            a.id.as_uuid(),
        )
        .con_detalle(auditar(&a)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Versión nueva de una tabla por tramos.
#[derive(Debug, Clone)]
pub struct NuevaTablaLaboral {
    pub tabla: TablaLaboral,
    pub vigente_desde: NaiveDate,
    pub fuente: String,
    pub tramos: Vec<Tramo>,
}

/// Agrega una versión completa de una tabla por tramos (la retención por salarios en UVT o el
/// fondo de solidaridad en salarios mínimos). Solo inserción: vale la de mayor `vigente_desde`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`;
/// [`domain::DominioError::ReglaLaboral`] si los tramos no empiezan en cero, no son contiguos o
/// el último no es abierto; [`AppError::Conflicto`] si ya hay una versión con esa fecha.
pub async fn crear_tabla_laboral<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nueva: NuevaTablaLaboral,
) -> AppResult<VersionTablaLaboral>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCatalogoLaboral + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let version = VersionTablaLaboral {
        tabla: nueva.tabla,
        vigente_desde: nueva.vigente_desde,
        fuente: texto("fuente", &nueva.fuente, 300)?,
        tramos: TablaTramos::new(nueva.tramos)?,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_tabla(&version).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tabla", version.tabla.as_str())?
        .con("vigente_desde", version.vigente_desde.to_string())?
        .con("tramos", version.tramos.tramos().len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TABLA, "tabla_laboral", uuid::Uuid::nil()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(version)
}

/// Catálogo laboral: versiones de los parámetros y administradoras.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogoLaboralRespuesta {
    /// En la plataforma, todas las versiones; en la entidad, las vigentes en la fecha.
    pub parametros: Vec<VersionParametroLaboral>,
    /// En la plataforma, todas; en la entidad, las activas.
    pub administradoras: Vec<Administradora>,
    /// En la plataforma, todas las versiones; en la entidad, la vigente de cada tabla.
    pub tablas: Vec<VersionTablaLaboral>,
}

/// Catálogo completo (todas las versiones y administradoras) visto desde la plataforma.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`.
pub async fn catalogo_plataforma<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<CatalogoLaboralRespuesta>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCatalogoLaboral,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    Ok(CatalogoLaboralRespuesta {
        parametros: tx.parametros(None).await?,
        administradoras: tx.administradoras(false).await?,
        tablas: tx.tablas(None).await?,
    })
}

/// Catálogo vigente en la fecha (por defecto, hoy) con las administradoras activas: lo que la
/// entidad usa en sus contratos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::ModuloNoHabilitado`] sin nómina.
pub async fn catalogo_vigente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    fecha: Option<NaiveDate>,
) -> AppResult<CatalogoLaboralRespuesta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let fecha = fecha.unwrap_or_else(|| reloj.ahora().date_naive());
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    Ok(CatalogoLaboralRespuesta {
        parametros: tx.parametros(Some(fecha)).await?,
        administradoras: tx.administradoras(true).await?,
        tablas: tx.tablas(Some(fecha)).await?,
    })
}

/// Catálogo laboral vigente en la fecha, dentro de una transacción.
pub(crate) async fn catalogo_en<T: RepositorioCatalogoLaboral>(
    tx: &mut T,
    fecha: NaiveDate,
) -> AppResult<CatalogoLaboral> {
    let versiones: Vec<VersionParametro> = tx
        .parametros(Some(fecha))
        .await?
        .into_iter()
        .map(|v| VersionParametro {
            parametro: v.parametro,
            valor: v.valor,
            vigente_desde: v.vigente_desde,
        })
        .collect();
    Ok(CatalogoLaboral::vigente_en(&versiones, fecha))
}
