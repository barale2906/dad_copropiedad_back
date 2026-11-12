//! Censo de la copropiedad (plan 07a, Ley 675 art. 51): residencias, vehículos, mascotas,
//! autoservicio con aprobación, alertas, libros de propietarios y residentes, y personas.
//!
//! Reglas:
//! - Un propietario residente es titular vigente del bien; un tenedor lo autoriza un titular
//!   vigente. Cada bien tiene a lo sumo un responsable vigente.
//! - Residentes permanentes con cuenta reciben el rol derivado `residente` (origen
//!   `residencia`); terminar la residencia lo vence **en la misma transacción** (invariante 5).
//! - Residentes y propietarios proponen cambios de su bien; la administración los aprueba.
//! - Los datos sensibles (fecha de nacimiento, movilidad reducida) solo se ven y editan con
//!   `personas.datos_sensibles.ver`.

use chrono::{NaiveDate, Utc};
use domain::{
    Ambito, ArchivoId, BienPrivadoId, CalidadResidencia, CambioCensoId, ConjuntoPermisos,
    DireccionCorreo, Documento, EspecieMascota, EstadoCambioCenso, EstadoMascota, MascotaId,
    OrigenAsignacion, Permiso, PersonaId, Placa, ResidenciaId, Telefono, TipoCambioCenso,
    TipoDocumento, TipoVehiculo, VehiculoId,
    censo::{AlertaMascota, VencimientosMascota, alertas_mascota, es_menor, exigir_placa},
    copropiedad::{texto_obligatorio, texto_opcional},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        roles::{SolicitudAsignacion, asignar_en},
        titulares::{RefPersona, resolver_persona},
    },
    puertos::{
        RegistroAuditoria, Reloj, RepositorioBienesPrivados, RepositorioPersonas, RepositorioRoles,
        RepositorioTitulares, Transaccion, UnidadDeTrabajo,
        censo::{
            CambioCenso, FilaLibroPropietarios, FilaLibroResidentes, Mascota, RepositorioCenso,
            Residencia, Vehiculo,
        },
        copropiedades::Persona,
    },
};

pub(crate) const ACCION_RESIDENCIA: AccionAuditoria =
    AccionAuditoria::new("censo.residencia.crear");
pub(crate) const ACCION_TERMINAR_RESIDENCIA: AccionAuditoria =
    AccionAuditoria::new("censo.residencia.terminar");
pub(crate) const ACCION_VEHICULO: AccionAuditoria = AccionAuditoria::new("censo.vehiculo.crear");
pub(crate) const ACCION_RETIRAR_VEHICULO: AccionAuditoria =
    AccionAuditoria::new("censo.vehiculo.retirar");
pub(crate) const ACCION_MASCOTA: AccionAuditoria = AccionAuditoria::new("censo.mascota.crear");
pub(crate) const ACCION_EDITAR_MASCOTA: AccionAuditoria =
    AccionAuditoria::new("censo.mascota.editar");
pub(crate) const ACCION_PROPONER: AccionAuditoria = AccionAuditoria::new("censo.cambio.proponer");
pub(crate) const ACCION_APROBAR: AccionAuditoria = AccionAuditoria::new("censo.cambio.aprobar");
pub(crate) const ACCION_RECHAZAR: AccionAuditoria = AccionAuditoria::new("censo.cambio.rechazar");
pub(crate) const ACCION_EDITAR_PERSONA: AccionAuditoria =
    AccionAuditoria::new("personas.persona.editar");

/// Código del rol plantilla que reciben los residentes permanentes con cuenta.
pub const CODIGO_RESIDENTE: &str = "residente";

/// El rol derivado lo asigna el sistema a partir del hecho registrado, no quien registra.
fn ctx_sistema(ctx: &Contexto) -> Contexto {
    Contexto {
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
        ..ctx.clone()
    }
}

async fn exigir_bien_activo<T: RepositorioBienesPrivados>(
    tx: &mut T,
    bien: BienPrivadoId,
) -> AppResult<()> {
    tx.bien(bien)
        .await?
        .filter(|b| b.activo)
        .map(|_| ())
        .ok_or(AppError::NoEncontrado("bien privado"))
}

// ---------------------------------------------------------------------------------------------
// Residencias
// ---------------------------------------------------------------------------------------------

/// Datos de una residencia nueva.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuevaResidencia {
    pub persona: RefPersona,
    pub calidad: CalidadResidencia,
    #[serde(default)]
    pub es_responsable: bool,
    /// Titular vigente que autoriza al tenedor (obligatorio para tenedores).
    #[serde(default)]
    pub autorizado_por: Option<PersonaId>,
    #[serde(default)]
    pub contrato_archivo_id: Option<ArchivoId>,
}

async fn registrar_residencia_en<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    nueva: NuevaResidencia,
) -> AppResult<Residencia>
where
    T: RepositorioCenso
        + RepositorioPersonas
        + RepositorioTitulares
        + RepositorioBienesPrivados
        + RepositorioRoles
        + RegistroAuditoria,
{
    exigir_bien_activo(tx, bien).await?;
    let titulares: Vec<PersonaId> = tx
        .titulares_vigentes(bien)
        .await?
        .into_iter()
        .map(|t| t.persona_id)
        .collect();
    let (persona, _) = resolver_persona(tx, nueva.persona).await?;
    match nueva.calidad {
        CalidadResidencia::PropietarioResidente if !titulares.contains(&persona.id) => {
            return Err(AppError::Validacion(
                "un propietario residente debe ser titular vigente del bien privado".into(),
            ));
        }
        CalidadResidencia::Tenedor
            if !nueva.autorizado_por.is_some_and(|p| titulares.contains(&p)) =>
        {
            return Err(AppError::Validacion(
                "un tenedor lo autoriza un titular vigente del bien privado (`autorizado_por`)"
                    .into(),
            ));
        }
        _ => {}
    }
    let vigentes = tx.residencias(Some(bien), false).await?;
    if vigentes.iter().any(|r| r.persona_id == persona.id) {
        return Err(AppError::Conflicto(
            "la persona ya reside en ese bien privado".into(),
        ));
    }
    if nueva.es_responsable && vigentes.iter().any(|r| r.es_responsable) {
        return Err(AppError::Conflicto(
            "el bien privado ya tiene un responsable vigente".into(),
        ));
    }
    let ahora = reloj.ahora();
    let residencia = Residencia {
        id: ResidenciaId::nuevo(),
        bien_privado_id: bien,
        persona_id: persona.id,
        calidad: nueva.calidad,
        es_responsable: nueva.es_responsable,
        autorizado_por: nueva.autorizado_por,
        contrato_archivo_id: nueva.contrato_archivo_id,
        vigente_desde: ahora,
        vigente_hasta: None,
        reservas_autorizadas: false,
    };
    tx.insertar_residencia(&residencia).await?;
    if let (true, Some(cuenta)) = (residencia.calidad.recibe_rol_residente(), persona.cuenta_id) {
        let rol = tx
            .rol_por_codigo(CODIGO_RESIDENTE)
            .await?
            .ok_or(AppError::NoEncontrado("rol residente"))?;
        asignar_en(
            tx,
            reloj,
            &ctx_sistema(ctx),
            SolicitudAsignacion {
                cuenta_id: cuenta,
                rol_id: rol.id,
                vigente_desde: Some(ahora),
                vigente_hasta: None,
                origen: OrigenAsignacion::Residencia,
                origen_id: Some(residencia.id.as_uuid()),
            },
        )
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.to_string())?
        .con("calidad", residencia.calidad.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RESIDENCIA, "residencia", residencia.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(residencia)
}

/// Registra que una persona reside en un bien privado.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`].
/// - [`AppError::Validacion`] si un propietario residente no es titular o un tenedor no está
///   autorizado por un titular.
/// - [`AppError::Conflicto`] si ya reside allí o el bien ya tiene responsable.
pub async fn registrar_residencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    nueva: NuevaResidencia,
) -> AppResult<Residencia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso
        + RepositorioPersonas
        + RepositorioTitulares
        + RepositorioBienesPrivados
        + RepositorioRoles
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let r = registrar_residencia_en(&mut tx, reloj, ctx, bien, nueva).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Termina una residencia y vence su rol derivado en la misma transacción.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`].
pub async fn terminar_residencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ResidenciaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let r = tx
        .residencia(id)
        .await?
        .ok_or(AppError::NoEncontrado("residencia"))?;
    if r.vigente_hasta.is_some() {
        return Ok(());
    }
    tx.terminar_residencia(id, ahora).await?;
    let vencidos = tx
        .terminar_por_origen(OrigenAsignacion::Residencia, id.as_uuid(), ahora)
        .await?;
    let detalle = DetalleAuditoria::vacio().con("roles_vencidos", vencidos)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TERMINAR_RESIDENCIA, "residencia", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Residencia con los datos de la persona (datos sensibles ocultos según el permiso).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidenciaConPersona {
    pub residencia: Residencia,
    pub persona: PersonaVista,
}

/// Residencias (de un bien o de todos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.ver`.
pub async fn listar_residencias<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
    incluir_terminadas: bool,
) -> AppResult<Vec<ResidenciaConPersona>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RepositorioPersonas,
{
    ctx.exigir(Permiso::CensoVer)?;
    let hoy = reloj.ahora().date_naive();
    let mut tx = unidad.iniciar(ctx).await?;
    let mut lista = Vec::new();
    for r in tx.residencias(bien, incluir_terminadas).await? {
        let persona = tx
            .persona(r.persona_id)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
        lista.push(ResidenciaConPersona {
            residencia: r,
            persona: PersonaVista::para(ctx, persona, hoy),
        });
    }
    Ok(lista)
}

// ---------------------------------------------------------------------------------------------
// Vehículos
// ---------------------------------------------------------------------------------------------

/// Datos de un vehículo nuevo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuevoVehiculo {
    pub tipo: TipoVehiculo,
    #[serde(default)]
    pub placa: Option<Placa>,
    #[serde(default)]
    pub marca: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// Residente o propietario a quien pertenece.
    #[serde(default)]
    pub persona_id: Option<PersonaId>,
    #[serde(default)]
    pub tarjeta_archivo_id: Option<ArchivoId>,
}

async fn registrar_vehiculo_en<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    bien: BienPrivadoId,
    n: NuevoVehiculo,
) -> AppResult<Vehiculo>
where
    T: RepositorioCenso + RepositorioBienesPrivados + RepositorioPersonas + RegistroAuditoria,
{
    exigir_placa(n.tipo, n.placa.as_ref())?;
    exigir_bien_activo(tx, bien).await?;
    if let Some(p) = n.persona_id {
        tx.persona(p)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
    }
    let v = Vehiculo {
        id: VehiculoId::nuevo(),
        bien_privado_id: bien,
        persona_id: n.persona_id,
        tipo: n.tipo,
        placa: n.placa,
        marca: texto_opcional("marca", n.marca.as_deref(), 60)?,
        color: texto_opcional("color", n.color.as_deref(), 40)?,
        tarjeta_archivo_id: n.tarjeta_archivo_id,
        vigente_desde: reloj.ahora(),
        vigente_hasta: None,
    };
    tx.insertar_vehiculo(&v).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.to_string())?
        .con("tipo", v.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_VEHICULO, "vehiculo", v.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(v)
}

/// Registra un vehículo de un bien privado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::Dominio`] si un carro o moto no
/// trae placa; [`AppError::Conflicto`] si la placa ya está vigente en la copropiedad.
pub async fn registrar_vehiculo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    nuevo: NuevoVehiculo,
) -> AppResult<Vehiculo>
where
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioCenso + RepositorioBienesPrivados + RepositorioPersonas + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let v = registrar_vehiculo_en(&mut tx, reloj, bien, nuevo).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// Retira un vehículo (deja de estar vigente; su placa queda libre).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`].
pub async fn retirar_vehiculo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VehiculoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let v = tx
        .vehiculo(id)
        .await?
        .ok_or(AppError::NoEncontrado("vehículo"))?;
    if v.vigente_hasta.is_none() {
        tx.retirar_vehiculo(id, reloj.ahora()).await?;
        tx.registrar(EventoAuditoria::ok(
            ACCION_RETIRAR_VEHICULO,
            "vehiculo",
            id.as_uuid(),
        ))
        .await?;
    }
    tx.confirmar().await
}

/// Vehículos (de un bien, por placa o todos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.ver`.
pub async fn listar_vehiculos<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
    placa: Option<Placa>,
    incluir_retirados: bool,
) -> AppResult<Vec<Vehiculo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    ctx.exigir(Permiso::CensoVer)?;
    unidad
        .iniciar(ctx)
        .await?
        .vehiculos(bien, placa.as_ref(), incluir_retirados)
        .await
}

// ---------------------------------------------------------------------------------------------
// Mascotas
// ---------------------------------------------------------------------------------------------

/// Datos de una mascota nueva.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuevaMascota {
    pub especie: EspecieMascota,
    pub nombre: String,
    #[serde(default)]
    pub raza: Option<String>,
    #[serde(default)]
    pub potencialmente_peligrosa: bool,
    #[serde(default)]
    pub poliza_numero: Option<String>,
    #[serde(default)]
    pub poliza_vence: Option<NaiveDate>,
    #[serde(default)]
    pub poliza_archivo_id: Option<ArchivoId>,
    #[serde(default)]
    pub vacunas_vencen: Option<NaiveDate>,
    #[serde(default)]
    pub carne_archivo_id: Option<ArchivoId>,
    #[serde(default)]
    pub esterilizada: bool,
    #[serde(default)]
    pub foto_archivo_id: Option<ArchivoId>,
    #[serde(default)]
    pub responsable_id: Option<PersonaId>,
}

async fn registrar_mascota_en<T>(
    tx: &mut T,
    bien: BienPrivadoId,
    n: NuevaMascota,
) -> AppResult<Mascota>
where
    T: RepositorioCenso + RepositorioBienesPrivados + RepositorioPersonas + RegistroAuditoria,
{
    exigir_bien_activo(tx, bien).await?;
    if let Some(p) = n.responsable_id {
        tx.persona(p)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
    }
    let m = Mascota {
        id: MascotaId::nuevo(),
        bien_privado_id: bien,
        responsable_id: n.responsable_id,
        especie: n.especie,
        nombre: texto_obligatorio("nombre", &n.nombre, 60)?,
        raza: texto_opcional("raza", n.raza.as_deref(), 60)?,
        potencialmente_peligrosa: n.potencialmente_peligrosa,
        poliza_numero: texto_opcional("poliza_numero", n.poliza_numero.as_deref(), 60)?,
        poliza_vence: n.poliza_vence,
        poliza_archivo_id: n.poliza_archivo_id,
        vacunas_vencen: n.vacunas_vencen,
        carne_archivo_id: n.carne_archivo_id,
        esterilizada: n.esterilizada,
        foto_archivo_id: n.foto_archivo_id,
        estado: EstadoMascota::Activa,
    };
    tx.insertar_mascota(&m).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.to_string())?
        .con("especie", m.especie.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MASCOTA, "mascota", m.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(m)
}

/// Registra una mascota de un bien privado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si un texto es inválido.
pub async fn registrar_mascota<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: BienPrivadoId,
    nueva: NuevaMascota,
) -> AppResult<Mascota>
where
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioCenso + RepositorioBienesPrivados + RepositorioPersonas + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let m = registrar_mascota_en(&mut tx, bien, nueva).await?;
    tx.confirmar().await?;
    Ok(m)
}

/// Cambios a una mascota. `None` = sin cambio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosMascota {
    pub raza: Option<String>,
    pub potencialmente_peligrosa: Option<bool>,
    pub poliza_numero: Option<String>,
    pub poliza_vence: Option<NaiveDate>,
    pub poliza_archivo_id: Option<ArchivoId>,
    pub vacunas_vencen: Option<NaiveDate>,
    pub carne_archivo_id: Option<ArchivoId>,
    pub esterilizada: Option<bool>,
    pub foto_archivo_id: Option<ArchivoId>,
    pub responsable_id: Option<PersonaId>,
    /// Fallecida o trasladada (desde activa).
    pub estado: Option<EstadoMascota>,
}

/// Actualiza una mascota (vacunas, póliza, estado…).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si el cambio de estado no está permitido.
pub async fn actualizar_mascota<U>(
    unidad: &U,
    ctx: &Contexto,
    id: MascotaId,
    c: CambiosMascota,
) -> AppResult<Mascota>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut m = tx
        .mascota(id)
        .await?
        .ok_or(AppError::NoEncontrado("mascota"))?;
    if let Some(e) = c.estado {
        m.estado = m.estado.transicionar(e)?;
    }
    if let Some(v) = c.raza {
        m.raza = texto_opcional("raza", Some(&v), 60)?;
    }
    if let Some(v) = c.poliza_numero {
        m.poliza_numero = texto_opcional("poliza_numero", Some(&v), 60)?;
    }
    if let Some(v) = c.potencialmente_peligrosa {
        m.potencialmente_peligrosa = v;
    }
    if let Some(v) = c.esterilizada {
        m.esterilizada = v;
    }
    m.poliza_vence = c.poliza_vence.or(m.poliza_vence);
    m.poliza_archivo_id = c.poliza_archivo_id.or(m.poliza_archivo_id);
    m.vacunas_vencen = c.vacunas_vencen.or(m.vacunas_vencen);
    m.carne_archivo_id = c.carne_archivo_id.or(m.carne_archivo_id);
    m.foto_archivo_id = c.foto_archivo_id.or(m.foto_archivo_id);
    m.responsable_id = c.responsable_id.or(m.responsable_id);
    tx.actualizar_mascota(&m).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_MASCOTA,
        "mascota",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(m)
}

/// Mascotas (de un bien o todas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.ver`.
pub async fn listar_mascotas<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
    solo_activas: bool,
) -> AppResult<Vec<Mascota>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    ctx.exigir(Permiso::CensoVer)?;
    unidad
        .iniciar(ctx)
        .await?
        .mascotas(bien, solo_activas)
        .await
}

// ---------------------------------------------------------------------------------------------
// Autoservicio: propuestas de cambio
// ---------------------------------------------------------------------------------------------

/// Lo que propone un residente o propietario para su bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropuestaCenso {
    Residencia(NuevaResidencia),
    Vehiculo(NuevoVehiculo),
    Mascota(NuevaMascota),
}

impl PropuestaCenso {
    /// Tipo de cambio que se propone (según el registro que crearía).
    pub const fn tipo(&self) -> TipoCambioCenso {
        match self {
            Self::Residencia(_) => TipoCambioCenso::Residencia,
            Self::Vehiculo(_) => TipoCambioCenso::Vehiculo,
            Self::Mascota(_) => TipoCambioCenso::Mascota,
        }
    }

    fn a_json(&self) -> AppResult<serde_json::Value> {
        match self {
            Self::Residencia(r) => serde_json::to_value(r),
            Self::Vehiculo(v) => serde_json::to_value(v),
            Self::Mascota(m) => serde_json::to_value(m),
        }
        .map_err(AppError::infra)
    }

    fn desde_json(tipo: TipoCambioCenso, datos: serde_json::Value) -> AppResult<Self> {
        match tipo {
            TipoCambioCenso::Residencia => serde_json::from_value(datos).map(Self::Residencia),
            TipoCambioCenso::Vehiculo => serde_json::from_value(datos).map(Self::Vehiculo),
            TipoCambioCenso::Mascota => serde_json::from_value(datos).map(Self::Mascota),
        }
        .map_err(AppError::infra)
    }
}

/// Un residente o propietario propone un cambio del censo de su bien privado. Queda pendiente
/// hasta que la administración lo apruebe. Una propuesta nunca vincula cuentas (eso lo hace la
/// administración).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta; [`AppError::SinPermiso`] si la cuenta no es titular
/// ni residente vigente del bien (ni tiene `censo.editar`).
pub async fn proponer_cambio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    mut propuesta: PropuestaCenso,
) -> AppResult<CambioCenso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RepositorioBienesPrivados + RegistroAuditoria,
{
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_bien_activo(&mut tx, bien).await?;
    if !tx.cuenta_vinculada_a_bien(cuenta, bien).await? {
        ctx.exigir(Permiso::CensoEditar)?;
    }
    if let PropuestaCenso::Residencia(NuevaResidencia {
        persona: RefPersona::Nueva(d),
        ..
    }) = &mut propuesta
    {
        d.cuenta_id = None;
    }
    if let PropuestaCenso::Vehiculo(v) = &propuesta {
        exigir_placa(v.tipo, v.placa.as_ref())?;
    }
    let cambio = CambioCenso {
        id: CambioCensoId::nuevo(),
        bien_privado_id: bien,
        tipo: propuesta.tipo(),
        datos: propuesta.a_json()?,
        estado: EstadoCambioCenso::Pendiente,
        propuesto_por: cuenta,
        propuesto_en: reloj.ahora(),
        revisado_por: None,
        revisado_en: None,
        motivo: None,
        resultado_id: None,
    };
    tx.insertar_cambio(&cambio).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.to_string())?
        .con("tipo", cambio.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PROPONER, "cambio_censo", cambio.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(cambio)
}

/// Aprueba una propuesta: crea la residencia, el vehículo o la mascota en la misma transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `censo.aprobar`; [`AppError::NoEncontrado`].
/// - [`AppError::Dominio`] si ya estaba resuelta; los errores del registro que crea (p. ej.
///   [`AppError::Conflicto`] por una placa repetida).
pub async fn aprobar_cambio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CambioCensoId,
) -> AppResult<CambioCenso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso
        + RepositorioPersonas
        + RepositorioTitulares
        + RepositorioBienesPrivados
        + RepositorioRoles
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoAprobar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut cambio = tx
        .cambio(id)
        .await?
        .ok_or(AppError::NoEncontrado("cambio del censo"))?;
    cambio.estado = cambio.estado.transicionar(EstadoCambioCenso::Aprobado)?;
    let bien = cambio.bien_privado_id;
    let resultado = match PropuestaCenso::desde_json(cambio.tipo, cambio.datos.clone())? {
        PropuestaCenso::Residencia(r) => registrar_residencia_en(&mut tx, reloj, ctx, bien, r)
            .await?
            .id
            .as_uuid(),
        PropuestaCenso::Vehiculo(v) => registrar_vehiculo_en(&mut tx, reloj, bien, v)
            .await?
            .id
            .as_uuid(),
        PropuestaCenso::Mascota(m) => registrar_mascota_en(&mut tx, bien, m).await?.id.as_uuid(),
    };
    cambio.revisado_por = ctx.actor_id;
    cambio.revisado_en = Some(reloj.ahora());
    cambio.resultado_id = Some(resultado);
    tx.resolver_cambio(&cambio).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_APROBAR,
        "cambio_censo",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(cambio)
}

/// Rechaza una propuesta con un motivo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si ya estaba resuelta.
pub async fn rechazar_cambio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CambioCensoId,
    motivo: &str,
) -> AppResult<CambioCenso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RegistroAuditoria,
{
    ctx.exigir(Permiso::CensoAprobar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut cambio = tx
        .cambio(id)
        .await?
        .ok_or(AppError::NoEncontrado("cambio del censo"))?;
    cambio.estado = cambio.estado.transicionar(EstadoCambioCenso::Rechazado)?;
    cambio.motivo = Some(texto_obligatorio("motivo", motivo, 500)?);
    cambio.revisado_por = ctx.actor_id;
    cambio.revisado_en = Some(reloj.ahora());
    tx.resolver_cambio(&cambio).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_RECHAZAR,
        "cambio_censo",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(cambio)
}

/// Propuestas de la copropiedad (con `censo.aprobar`) o las propias de la cuenta.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta para `solo_mias`; [`AppError::SinPermiso`] sin
/// `censo.aprobar` para ver las de todos.
pub async fn listar_cambios<U>(
    unidad: &U,
    ctx: &Contexto,
    estado: Option<EstadoCambioCenso>,
    solo_mias: bool,
) -> AppResult<Vec<CambioCenso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    let propuesto_por = if solo_mias {
        Some(ctx.actor_id.ok_or(AppError::NoAutenticado)?)
    } else {
        ctx.exigir(Permiso::CensoAprobar)?;
        None
    };
    unidad
        .iniciar(ctx)
        .await?
        .cambios(estado, propuesto_por)
        .await
}

// ---------------------------------------------------------------------------------------------
// Libros y alertas
// ---------------------------------------------------------------------------------------------

/// Libro de propietarios a la fecha de corte (titulares vigentes entonces, con el coeficiente
/// del cuadro que regía).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.exportar`.
pub async fn libro_propietarios<U>(
    unidad: &U,
    ctx: &Contexto,
    corte: chrono::DateTime<Utc>,
) -> AppResult<Vec<FilaLibroPropietarios>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    ctx.exigir(Permiso::CensoExportar)?;
    unidad.iniciar(ctx).await?.libro_propietarios(corte).await
}

/// Libro de residentes a la fecha de corte.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.exportar`.
pub async fn libro_residentes<U>(
    unidad: &U,
    ctx: &Contexto,
    corte: chrono::DateTime<Utc>,
) -> AppResult<Vec<FilaLibroResidentes>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    ctx.exigir(Permiso::CensoExportar)?;
    unidad.iniciar(ctx).await?.libro_residentes(corte).await
}

/// Libro listo para imprimir: las columnas y celdas ya formateadas (las mismas del CSV).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibroPdf {
    pub titulo: String,
    pub corte: chrono::DateTime<Utc>,
    pub columnas: Vec<String>,
    pub filas: Vec<Vec<String>>,
}

/// PDF de un libro del censo (art. 51) con el nombre de la PH y la fecha de corte en su hora
/// local.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.exportar`; errores del generador.
pub async fn libro_en_pdf<U, G>(
    unidad: &U,
    reloj: &impl Reloj,
    generador: &G,
    ctx: &Contexto,
    libro: LibroPdf,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: crate::puertos::RepositorioCopropiedad,
    G: crate::puertos::GeneradorActas,
{
    ctx.exigir(Permiso::CensoExportar)?;
    let ficha =
        crate::puertos::RepositorioCopropiedad::ficha(&mut unidad.iniciar(ctx).await?).await?;
    let (nombre, tz) = ficha.map_or((String::new(), domain::reserva::Tz::America__Bogota), |f| {
        let tz = f
            .zona_horaria
            .parse()
            .unwrap_or(domain::reserva::Tz::America__Bogota);
        (f.nombre, tz)
    });
    let hora = |t| crate::puertos::actas::hora_local(t, tz);
    generador
        .tabla(&crate::puertos::actas::TablaPdf {
            titulo: libro.titulo,
            copropiedad: nombre,
            subtitulo: format!("Fecha de corte: {}", hora(libro.corte)),
            columnas: libro.columnas,
            filas: libro.filas,
            generada: hora(reloj.ahora()),
        })
        .await
}

/// Alertas del censo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertasCenso {
    /// Mascotas activas con vacunas o pólizas vencidas, por vencer o sin registrar.
    pub mascotas: Vec<(Mascota, Vec<AlertaMascota>)>,
    /// Residentes vigentes sin autorización de tratamiento para administración (Ley 1581).
    pub residentes_sin_autorizacion: Vec<PersonaId>,
}

/// Alertas del censo en la fecha del reloj.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.ver`.
pub async fn alertas_censo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<AlertasCenso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso,
{
    ctx.exigir(Permiso::CensoVer)?;
    let hoy = reloj.ahora().date_naive();
    let mut tx = unidad.iniciar(ctx).await?;
    let mascotas = tx
        .mascotas(None, true)
        .await?
        .into_iter()
        .filter_map(|m| {
            let alertas = alertas_mascota(
                &VencimientosMascota {
                    estado: m.estado,
                    potencialmente_peligrosa: m.potencialmente_peligrosa,
                    tiene_poliza: m.poliza_numero.is_some(),
                    poliza_vence: m.poliza_vence,
                    vacunas_vencen: m.vacunas_vencen,
                },
                hoy,
            );
            (!alertas.is_empty()).then_some((m, alertas))
        })
        .collect();
    Ok(AlertasCenso {
        mascotas,
        residentes_sin_autorizacion: tx.residentes_sin_autorizacion().await?,
    })
}

// ---------------------------------------------------------------------------------------------
// Personas
// ---------------------------------------------------------------------------------------------

/// Persona tal como la ve quien consulta: sin datos sensibles si no tiene
/// `personas.datos_sensibles.ver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaVista {
    pub persona: Persona,
    /// Menor de edad (si se conoce la fecha de nacimiento): sus datos tienen tratamiento
    /// especial.
    pub es_menor: Option<bool>,
    /// Se ocultaron la fecha de nacimiento y la movilidad reducida.
    pub datos_sensibles_ocultos: bool,
}

impl PersonaVista {
    fn para(ctx: &Contexto, mut persona: Persona, hoy: NaiveDate) -> Self {
        let es_menor = persona.fecha_nacimiento.map(|f| es_menor(f, hoy));
        let ocultar = ctx.exigir(Permiso::PersonasDatosSensiblesVer).is_err();
        if ocultar {
            persona.fecha_nacimiento = None;
            persona.movilidad_reducida = false;
        }
        Self {
            persona,
            es_menor,
            datos_sensibles_ocultos: ocultar,
        }
    }
}

/// Personas de la copropiedad (búsqueda por nombre o documento).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `personas.ver`.
pub async fn listar_personas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    texto: Option<String>,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<PersonaVista>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioPersonas,
{
    ctx.exigir(Permiso::PersonasVer)?;
    let hoy = reloj.ahora().date_naive();
    let pagina = unidad
        .iniciar(ctx)
        .await?
        .listar_personas(
            texto.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            pagina,
        )
        .await?;
    Ok(pagina.mapear(|p| PersonaVista::para(ctx, p, hoy)))
}

/// Una persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `personas.ver`; [`AppError::NoEncontrado`].
pub async fn ver_persona<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PersonaId,
) -> AppResult<PersonaVista>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioPersonas,
{
    ctx.exigir(Permiso::PersonasVer)?;
    let p = unidad
        .iniciar(ctx)
        .await?
        .persona(id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    Ok(PersonaVista::para(ctx, p, reloj.ahora().date_naive()))
}

/// Cambios a una persona. `None` = sin cambio; en los textos opcionales, `Some("")` borra.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosPersona {
    pub nombre: Option<String>,
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Option<Documento>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub contacto_emergencia_nombre: Option<String>,
    pub contacto_emergencia_telefono: Option<Telefono>,
    /// Dato sensible: exige `personas.datos_sensibles.ver`.
    pub fecha_nacimiento: Option<NaiveDate>,
    /// Dato sensible (salud): exige `personas.datos_sensibles.ver` y el consentimiento del
    /// titular.
    pub movilidad_reducida: Option<bool>,
}

/// Actualiza los datos de una persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `personas.editar` (o sin `personas.datos_sensibles.ver` para
/// los datos sensibles); [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si el documento ya
/// lo tiene otra persona.
pub async fn actualizar_persona<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PersonaId,
    c: CambiosPersona,
) -> AppResult<PersonaVista>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioPersonas + RegistroAuditoria,
{
    ctx.exigir(Permiso::PersonasEditar)?;
    if c.fecha_nacimiento.is_some() || c.movilidad_reducida.is_some() {
        ctx.exigir(Permiso::PersonasDatosSensiblesVer)?;
    }
    let hoy = reloj.ahora().date_naive();
    if c.fecha_nacimiento.is_some_and(|f| f > hoy) {
        return Err(AppError::Validacion(
            "la fecha de nacimiento no puede ser futura".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    let mut p = tx
        .persona(id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    if let Some(n) = c.nombre {
        p.nombre = texto_obligatorio("nombre", &n, 200)?;
    }
    if let Some(v) = c.contacto_emergencia_nombre {
        p.contacto_emergencia_nombre = texto_opcional("contacto_emergencia_nombre", Some(&v), 120)?;
    }
    p.tipo_documento = c.tipo_documento.or(p.tipo_documento);
    p.documento = c.documento.or(p.documento);
    p.telefono = c.telefono.or(p.telefono);
    p.correo = c.correo.or(p.correo);
    p.contacto_emergencia_telefono = c
        .contacto_emergencia_telefono
        .or(p.contacto_emergencia_telefono);
    p.fecha_nacimiento = c.fecha_nacimiento.or(p.fecha_nacimiento);
    if let Some(m) = c.movilidad_reducida {
        p.movilidad_reducida = m;
    }
    tx.actualizar_persona(&p).await?;
    // Los datos de la persona no se auditan: solo que cambió.
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_PERSONA,
        "persona",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(PersonaVista::para(ctx, p, hoy))
}

#[cfg(test)]
mod tests {
    use domain::{CuentaId, Porcentaje, TenantId, TipoPersona, TipoTitularidad};
    use rust_decimal::dec;

    use super::*;
    use crate::{
        casos_uso::{
            bienes_privados::{crear_bien_privado, tests::nuevo_bien},
            titulares::{DatosPersona, SolicitudTitular, registrar_titulares},
        },
        puertos::{RelojSistema, unidad_trabajo::memoria::UnidadEnMemoria},
    };

    struct Entorno {
        unidad: UnidadEnMemoria,
        ctx: Contexto,
        bien: BienPrivadoId,
        /// Titular del bien, con cuenta.
        dueno: PersonaId,
        cuenta_dueno: CuentaId,
    }

    fn nueva(doc: &str, cuenta: Option<CuentaId>) -> RefPersona {
        RefPersona::Nueva(DatosPersona {
            tipo: TipoPersona::Natural,
            nombre: format!("Persona {doc}"),
            documento: Documento::new(doc).expect("válido"),
            telefono: None,
            correo: None,
            cuenta_id: cuenta,
            tipo_documento: Some(TipoDocumento::Cc),
        })
    }

    async fn entorno() -> Entorno {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let bien = crear_bien_privado(&unidad, &ctx, nuevo_bien("101", None))
            .await
            .expect("bien")
            .id;
        let cuenta_dueno = CuentaId::nuevo();
        let t = registrar_titulares(
            &unidad,
            &RelojSistema,
            &ctx,
            bien,
            vec![SolicitudTitular {
                persona: nueva("111", Some(cuenta_dueno)),
                tipo: TipoTitularidad::Propietario,
                porcentaje: Porcentaje::new(dec!(100)).expect("válido"),
                es_representante: true,
            }],
        )
        .await
        .expect("titular");
        Entorno {
            dueno: t[0].persona_id,
            unidad,
            ctx,
            bien,
            cuenta_dueno,
        }
    }

    fn residencia(persona: RefPersona, calidad: CalidadResidencia) -> NuevaResidencia {
        NuevaResidencia {
            persona,
            calidad,
            es_responsable: false,
            autorizado_por: None,
            contrato_archivo_id: None,
        }
    }

    fn roles_vigentes(e: &Entorno, cuenta: CuentaId) -> usize {
        let despues = RelojSistema.ahora() + chrono::Duration::seconds(1);
        e.unidad
            .datos()
            .asignaciones
            .iter()
            .filter(|(_, a)| a.cuenta_id == cuenta && a.vigente_en(despues))
            .count()
    }

    #[tokio::test]
    async fn propietario_residente_debe_ser_titular_y_tenedor_autorizado() {
        let e = entorno().await;
        let r = registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            residencia(nueva("999", None), CalidadResidencia::PropietarioResidente),
        )
        .await;
        assert!(matches!(r, Err(AppError::Validacion(_))));

        let r = registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            residencia(nueva("222", None), CalidadResidencia::Tenedor),
        )
        .await;
        assert!(
            matches!(r, Err(AppError::Validacion(_))),
            "sin autorización"
        );

        registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            NuevaResidencia {
                autorizado_por: Some(e.dueno),
                es_responsable: true,
                ..residencia(nueva("222", None), CalidadResidencia::Tenedor)
            },
        )
        .await
        .expect("tenedor autorizado");
        let r = registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            NuevaResidencia {
                es_responsable: true,
                ..residencia(
                    RefPersona::Existente(e.dueno),
                    CalidadResidencia::PropietarioResidente,
                )
            },
        )
        .await;
        assert!(
            matches!(r, Err(AppError::Conflicto(_))),
            "un solo responsable"
        );
    }

    #[tokio::test]
    async fn terminar_la_residencia_vence_el_rol_residente() {
        let e = entorno().await;
        let cuenta = CuentaId::nuevo();
        let r = registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            residencia(nueva("333", Some(cuenta)), CalidadResidencia::Familiar),
        )
        .await
        .expect("familiar");
        assert_eq!(roles_vigentes(&e, cuenta), 1);
        terminar_residencia(&e.unidad, &RelojSistema, &e.ctx, r.id)
            .await
            .expect("termina");
        assert_eq!(roles_vigentes(&e, cuenta), 0);
    }

    #[tokio::test]
    async fn huespedes_y_empleados_no_reciben_rol() {
        let e = entorno().await;
        let cuenta = CuentaId::nuevo();
        registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            residencia(
                nueva("444", Some(cuenta)),
                CalidadResidencia::EmpleadoDomestico,
            ),
        )
        .await
        .expect("empleado");
        assert_eq!(roles_vigentes(&e, cuenta), 0);
    }

    fn carro(placa: Option<&str>) -> NuevoVehiculo {
        NuevoVehiculo {
            tipo: TipoVehiculo::Carro,
            placa: placa.map(|p| Placa::new(p).expect("válida")),
            marca: Some("Renault".into()),
            color: None,
            persona_id: None,
            tarjeta_archivo_id: None,
        }
    }

    #[tokio::test]
    async fn placa_obligatoria_y_unica() {
        let e = entorno().await;
        assert!(matches!(
            registrar_vehiculo(&e.unidad, &RelojSistema, &e.ctx, e.bien, carro(None)).await,
            Err(AppError::Dominio(_))
        ));
        let v = registrar_vehiculo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            carro(Some("abc-123")),
        )
        .await
        .expect("registra");
        assert!(matches!(
            registrar_vehiculo(
                &e.unidad,
                &RelojSistema,
                &e.ctx,
                e.bien,
                carro(Some("ABC123"))
            )
            .await,
            Err(AppError::Conflicto(_))
        ));
        retirar_vehiculo(&e.unidad, &RelojSistema, &e.ctx, v.id)
            .await
            .expect("retira");
        registrar_vehiculo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            carro(Some("ABC123")),
        )
        .await
        .expect("la placa quedó libre");
    }

    #[tokio::test]
    async fn el_propietario_propone_y_la_administracion_aprueba() {
        let e = entorno().await;
        let propietario = Contexto::de_cuenta(
            e.ctx.tenant_id,
            e.cuenta_dueno,
            [Permiso::CopropiedadDatosVer].into_iter().collect(),
        );
        let cambio = proponer_cambio(
            &e.unidad,
            &RelojSistema,
            &propietario,
            e.bien,
            PropuestaCenso::Vehiculo(carro(Some("XYZ987"))),
        )
        .await
        .expect("propone");
        assert!(
            e.unidad.datos().vehiculos.is_empty(),
            "nada cambia sin aprobar"
        );

        let extrano = Contexto::de_cuenta(
            e.ctx.tenant_id,
            CuentaId::nuevo(),
            ConjuntoPermisos::vacio(),
        );
        assert!(matches!(
            proponer_cambio(
                &e.unidad,
                &RelojSistema,
                &extrano,
                e.bien,
                PropuestaCenso::Vehiculo(carro(Some("XYZ988")))
            )
            .await,
            Err(AppError::SinPermiso(Permiso::CensoEditar))
        ));

        let aprobado = aprobar_cambio(&e.unidad, &RelojSistema, &e.ctx, cambio.id)
            .await
            .expect("aprueba");
        assert_eq!(aprobado.estado, EstadoCambioCenso::Aprobado);
        assert_eq!(e.unidad.datos().vehiculos.len(), 1);
        assert!(matches!(
            rechazar_cambio(&e.unidad, &RelojSistema, &e.ctx, cambio.id, "tarde").await,
            Err(AppError::Dominio(_))
        ));
        let mias = listar_cambios(&e.unidad, &propietario, None, true)
            .await
            .expect("mis cambios");
        assert_eq!(mias.len(), 1);
    }

    #[tokio::test]
    async fn una_propuesta_no_vincula_cuentas() {
        let e = entorno().await;
        let propietario =
            Contexto::de_cuenta(e.ctx.tenant_id, e.cuenta_dueno, ConjuntoPermisos::vacio());
        let otra = CuentaId::nuevo();
        let c = proponer_cambio(
            &e.unidad,
            &RelojSistema,
            &propietario,
            e.bien,
            PropuestaCenso::Residencia(residencia(
                nueva("555", Some(otra)),
                CalidadResidencia::Familiar,
            )),
        )
        .await
        .expect("propone");
        aprobar_cambio(&e.unidad, &RelojSistema, &e.ctx, c.id)
            .await
            .expect("aprueba");
        assert_eq!(roles_vigentes(&e, otra), 0);
    }

    #[tokio::test]
    async fn alertas_de_mascotas_y_residentes_sin_autorizacion() {
        let e = entorno().await;
        registrar_mascota(
            &e.unidad,
            &e.ctx,
            e.bien,
            NuevaMascota {
                especie: EspecieMascota::Perro,
                nombre: "Toby".into(),
                raza: Some("Pitbull".into()),
                potencialmente_peligrosa: true,
                poliza_numero: None,
                poliza_vence: None,
                poliza_archivo_id: None,
                vacunas_vencen: None,
                carne_archivo_id: None,
                esterilizada: false,
                foto_archivo_id: None,
                responsable_id: Some(e.dueno),
            },
        )
        .await
        .expect("mascota");
        registrar_residencia(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.bien,
            residencia(
                RefPersona::Existente(e.dueno),
                CalidadResidencia::PropietarioResidente,
            ),
        )
        .await
        .expect("residencia");
        let a = alertas_censo(&e.unidad, &RelojSistema, &e.ctx)
            .await
            .expect("alertas");
        assert_eq!(
            a.mascotas[0].1,
            vec![
                AlertaMascota::SinRegistroDeVacunas,
                AlertaMascota::SinPoliza
            ]
        );
        assert_eq!(a.residentes_sin_autorizacion, vec![e.dueno]);
    }

    #[tokio::test]
    async fn datos_sensibles_solo_con_su_permiso() {
        let e = entorno().await;
        let sin_sensibles = Contexto::de_cuenta(
            e.ctx.tenant_id,
            CuentaId::nuevo(),
            [Permiso::PersonasVer, Permiso::PersonasEditar]
                .into_iter()
                .collect(),
        );
        let nacimiento = NaiveDate::from_ymd_opt(2015, 3, 1).expect("fecha");
        assert!(matches!(
            actualizar_persona(
                &e.unidad,
                &RelojSistema,
                &sin_sensibles,
                e.dueno,
                CambiosPersona {
                    fecha_nacimiento: Some(nacimiento),
                    ..Default::default()
                }
            )
            .await,
            Err(AppError::SinPermiso(Permiso::PersonasDatosSensiblesVer))
        ));
        actualizar_persona(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.dueno,
            CambiosPersona {
                fecha_nacimiento: Some(nacimiento),
                ..Default::default()
            },
        )
        .await
        .expect("con permiso");
        let vista = ver_persona(&e.unidad, &RelojSistema, &sin_sensibles, e.dueno)
            .await
            .expect("ve");
        assert_eq!(vista.persona.fecha_nacimiento, None);
        assert_eq!(vista.es_menor, Some(true));
        assert!(vista.datos_sensibles_ocultos);
    }
}
