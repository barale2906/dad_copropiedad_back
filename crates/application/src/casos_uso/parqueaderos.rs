//! Parqueaderos (inventario y asignaciones) y sorteos verificables (plan 04, módulo
//! `bienes_comunes`).
//!
//! - Un parqueadero tiene a lo sumo una asignación en cada instante (la BD lo garantiza).
//! - Los de visitantes se reservan por franjas (con placa) con el motor de reservas.
//! - Un sorteo publica el compromiso (SHA-256 de la semilla) al crearse y revela la semilla al
//!   ejecutarse: cualquiera puede reproducir el resultado ([`domain::sorteo::ejecutar`]).

use chrono::{DateTime, Utc};
use domain::{
    AsignacionParqueaderoId, BienPrivadoId, EstadoRecurso, EstadoSorteo, Horario, ModoAsignacion,
    Modulo, NaturalezaParqueadero, OrigenAsignacionParqueadero, Permiso, PoliticaReserva,
    RecursoId, SorteoId, TipoParqueadero, TipoRecurso, VehiculoId,
    copropiedad::texto_obligatorio,
    sorteo::{CriteriosSorteo, Inscrito, Semilla, ejecutar},
};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{modulos::exigir_modulo_en, reservas::validar_textos_recurso},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioModulos, RepositorioReservas, Transaccion,
        UnidadDeTrabajo,
        reservas::{AsignacionParqueadero, DetalleParqueadero, InscripcionSorteo, Recurso, Sorteo},
    },
};

pub const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("parqueaderos.parqueadero.crear");
pub const ACCION_ASIGNAR: AccionAuditoria = AccionAuditoria::new("parqueaderos.asignacion.crear");
pub const ACCION_LIBERAR: AccionAuditoria =
    AccionAuditoria::new("parqueaderos.asignacion.terminar");
pub const ACCION_CREAR_SORTEO: AccionAuditoria = AccionAuditoria::new("sorteos.sorteo.crear");
pub const ACCION_INSCRIBIR: AccionAuditoria = AccionAuditoria::new("sorteos.inscripcion.crear");
pub const ACCION_EJECUTAR: AccionAuditoria = AccionAuditoria::new("sorteos.sorteo.ejecutar");

/// Datos de un parqueadero nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoParqueadero {
    pub codigo: String,
    pub tipo_vehiculo: TipoParqueadero,
    pub naturaleza: NaturalezaParqueadero,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub ubicacion: Option<String>,
    /// Solo visitantes: política de sus reservas (por defecto franjas inmediatas de 30 min a
    /// 4 h). Los demás no se reservan por franjas.
    pub politica: Option<PoliticaReserva>,
    pub horario: Horario,
}

/// Política por defecto de un parqueadero de visitantes.
pub fn politica_visitantes() -> PoliticaReserva {
    PoliticaReserva {
        modo: ModoAsignacion::FranjaInmediata,
        duracion_minima_min: 30,
        duracion_maxima_min: 240,
        granularidad_min: 30,
        anticipacion_maxima_dias: 7,
        ..PoliticaReserva::default()
    }
}

/// Crea un parqueadero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `parqueaderos.asignar`; [`AppError::Validacion`] si un privado
/// no indica su bien privado (o uno no privado lo indica); [`AppError::Conflicto`] si el código
/// existe.
pub async fn crear_parqueadero<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoParqueadero,
) -> AppResult<Recurso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ParqueaderosAsignar)?;
    if (n.naturaleza == NaturalezaParqueadero::Privado) != n.bien_privado_id.is_some() {
        return Err(AppError::Validacion(
            "un parqueadero privado indica su bien privado y los demás no".into(),
        ));
    }
    let politica = match (n.naturaleza, n.politica) {
        (NaturalezaParqueadero::Visitantes, Some(p)) => p.validar()?,
        (NaturalezaParqueadero::Visitantes, None) => politica_visitantes(),
        (NaturalezaParqueadero::Comunal, _) => PoliticaReserva {
            modo: ModoAsignacion::Sorteo,
            ..PoliticaReserva::default()
        },
        _ => PoliticaReserva {
            modo: ModoAsignacion::Libre,
            ..PoliticaReserva::default()
        },
    };
    let codigo = texto_obligatorio("codigo", &n.codigo, 20)?;
    let mut recurso = Recurso {
        id: RecursoId::nuevo(),
        tipo: TipoRecurso::Parqueadero,
        nombre: format!("Parqueadero {codigo}"),
        estado: EstadoRecurso::Disponible,
        ubicacion: n.ubicacion,
        descripcion: None,
        politica,
        horario: n.horario,
        bien_comun: None,
        parqueadero: Some(DetalleParqueadero {
            codigo,
            tipo_vehiculo: n.tipo_vehiculo,
            naturaleza: n.naturaleza,
            bien_privado_id: n.bien_privado_id,
        }),
    };
    validar_textos_recurso(&mut recurso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.insertar_recurso(&recurso).await?;
    let detalle = DetalleAuditoria::vacio().con("naturaleza", n.naturaleza.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "recurso", recurso.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(recurso)
}

/// Parqueadero con su asignación vigente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParqueaderoConAsignacion {
    pub recurso: Recurso,
    pub asignacion: Option<AsignacionParqueadero>,
}

/// Parqueaderos con su asignación vigente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `parqueaderos.ver`.
pub async fn listar_parqueaderos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ParqueaderoConAsignacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    ctx.exigir(Permiso::ParqueaderosVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let asignaciones = tx.asignaciones_parqueadero(None, None).await?;
    Ok(tx
        .recursos(Some(TipoRecurso::Parqueadero))
        .await?
        .into_iter()
        .map(|r| ParqueaderoConAsignacion {
            asignacion: asignaciones
                .iter()
                .find(|a| a.recurso_id == r.id && a.vigente_en(ahora))
                .cloned(),
            recurso: r,
        })
        .collect())
}

/// Solicitud de asignación de un parqueadero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudAsignacionParqueadero {
    pub recurso_id: RecursoId,
    pub bien_privado_id: BienPrivadoId,
    pub vehiculo_id: Option<VehiculoId>,
    pub origen: OrigenAsignacionParqueadero,
    /// `None` = desde ahora.
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Asigna un parqueadero a un bien privado por un periodo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `parqueaderos.asignar`; [`AppError::NoEncontrado`] si no es un
/// parqueadero; [`AppError::Validacion`] si es de visitantes, el origen es `sorteo` o la
/// vigencia está invertida; [`AppError::Conflicto`] si ya está asignado en ese periodo.
pub async fn asignar_parqueadero<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SolicitudAsignacionParqueadero,
) -> AppResult<AsignacionParqueadero>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ParqueaderosAsignar)?;
    if s.origen == OrigenAsignacionParqueadero::Sorteo {
        return Err(AppError::Validacion(
            "las asignaciones por sorteo las crea la ejecución del sorteo".into(),
        ));
    }
    let desde = s.desde.unwrap_or_else(|| reloj.ahora());
    if s.hasta.is_some_and(|h| h <= desde) {
        return Err(AppError::Validacion(
            "la vigencia termina antes de empezar".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let p = tx
        .recurso(s.recurso_id)
        .await?
        .and_then(|r| r.parqueadero)
        .ok_or(AppError::NoEncontrado("parqueadero"))?;
    if p.naturaleza == NaturalezaParqueadero::Visitantes {
        return Err(AppError::Validacion(
            "los parqueaderos de visitantes se reservan, no se asignan".into(),
        ));
    }
    let a = AsignacionParqueadero {
        id: AsignacionParqueaderoId::nuevo(),
        recurso_id: s.recurso_id,
        bien_privado_id: s.bien_privado_id,
        vehiculo_id: s.vehiculo_id,
        origen: s.origen,
        desde,
        hasta: s.hasta,
        sorteo_id: None,
    };
    tx.insertar_asignacion_parqueadero(&a).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", a.bien_privado_id.to_string())?
        .con("origen", a.origen.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ASIGNAR, "asignacion_parqueadero", a.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Libera un parqueadero (termina la asignación desde ahora).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `parqueaderos.asignar`; [`AppError::NoEncontrado`].
pub async fn liberar_parqueadero<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AsignacionParqueaderoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ParqueaderosAsignar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.asignacion_parqueadero(id)
        .await?
        .ok_or(AppError::NoEncontrado("asignación"))?;
    tx.terminar_asignacion_parqueadero(id, reloj.ahora())
        .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_LIBERAR,
        "asignacion_parqueadero",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Sorteos
// ---------------------------------------------------------------------------------------------

/// Datos de un sorteo nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoSorteo {
    pub nombre: String,
    /// Parqueaderos comunales que se adjudican.
    pub recursos: Vec<RecursoId>,
    /// Periodo de las asignaciones (p. ej. 6 meses).
    pub vigencia_desde: DateTime<Utc>,
    pub vigencia_hasta: DateTime<Utc>,
    pub criterios: CriteriosSorteo,
}

/// Crea un sorteo con una semilla aleatoria y publica su compromiso.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `sorteos.gestionar`; [`AppError::Validacion`] si no hay
/// parqueaderos comunales o la vigencia está invertida.
pub async fn crear_sorteo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoSorteo,
) -> AppResult<Sorteo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::SorteosGestionar)?;
    if n.recursos.is_empty() || n.vigencia_hasta <= n.vigencia_desde {
        return Err(AppError::Validacion(
            "indique al menos un parqueadero y una vigencia válida".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    for id in &n.recursos {
        let comunal = tx
            .recurso(*id)
            .await?
            .and_then(|r| r.parqueadero)
            .is_some_and(|p| p.naturaleza == NaturalezaParqueadero::Comunal);
        if !comunal {
            return Err(AppError::Validacion(
                "solo se sortean parqueaderos comunales".into(),
            ));
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| AppError::infra(std::io::Error::other(e.to_string())))?;
    let semilla = Semilla::desde_bytes(bytes);
    let sorteo = Sorteo {
        id: SorteoId::nuevo(),
        nombre: texto_obligatorio("nombre", &n.nombre, 120)?,
        recursos: n.recursos,
        vigencia_desde: n.vigencia_desde,
        vigencia_hasta: n.vigencia_hasta,
        estado: EstadoSorteo::Abierto,
        compromiso: semilla.compromiso(),
        semilla,
        criterios: n.criterios,
        resultado: None,
        creado_en: reloj.ahora(),
        ejecutado_en: None,
    };
    tx.insertar_sorteo(&sorteo).await?;
    let detalle = DetalleAuditoria::vacio().con("compromiso", sorteo.compromiso.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_SORTEO, "sorteo", sorteo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(sorteo)
}

/// Inscribe un bien privado en un sorteo abierto. Lo hace quien tiene vínculo con el bien
/// (titular o residente) o la administración (`sorteos.gestionar`).
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está abierto; [`AppError::SinPermiso`]
/// sin vínculo ni permiso; [`AppError::Conflicto`] si ya está inscrito.
pub async fn inscribirse_sorteo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    sorteo: SorteoId,
    bien: BienPrivadoId,
    discapacidad: bool,
) -> AppResult<InscripcionSorteo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let s = tx
        .sorteo(sorteo)
        .await?
        .ok_or(AppError::NoEncontrado("sorteo"))?;
    if s.estado != EstadoSorteo::Abierto {
        return Err(AppError::Conflicto("el sorteo ya se ejecutó".into()));
    }
    let vinculado = match ctx.actor_id {
        Some(c) => tx.vinculo_solicitante(c, bien).await?.is_some(),
        None => false,
    };
    if !vinculado {
        ctx.exigir(Permiso::SorteosGestionar)?;
    }
    let i = InscripcionSorteo {
        sorteo_id: sorteo,
        bien_privado_id: bien,
        discapacidad,
        inscrito_por: ctx.actor_id,
        inscrito_en: reloj.ahora(),
    };
    tx.insertar_inscripcion(&i).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_INSCRIBIR,
        "sorteo",
        sorteo.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(i)
}

/// Resultado de un sorteo ejecutado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoSorteo {
    pub sorteo: Sorteo,
    pub asignaciones: Vec<AsignacionParqueadero>,
}

/// Ejecuta el sorteo: revela la semilla, ordena a los inscritos (prioridades y luego la
/// semilla) y asigna los parqueaderos a los primeros por el periodo del sorteo. Quien se inscribió
/// en el sorteo ejecutado anterior y no salió favorecido suma los puntos de "no favorecido".
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `sorteos.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya se ejecutó o algún parqueadero ya está asignado en el periodo.
pub async fn ejecutar_sorteo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: SorteoId,
) -> AppResult<ResultadoSorteo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::SorteosGestionar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let mut s = tx
        .sorteo(id)
        .await?
        .ok_or(AppError::NoEncontrado("sorteo"))?;
    if s.estado != EstadoSorteo::Abierto {
        return Err(AppError::Conflicto("el sorteo ya se ejecutó".into()));
    }
    // Sorteo ejecutado anterior: sus inscritos no favorecidos ganan prioridad.
    let anterior = tx
        .sorteos()
        .await?
        .into_iter()
        .filter(|x| x.estado == EstadoSorteo::Ejecutado && x.creado_en < s.creado_en)
        .max_by_key(|x| x.creado_en);
    let no_favorecidos: Vec<BienPrivadoId> = match &anterior {
        Some(a) => {
            let cupos = a.recursos.len();
            let favorecidos: Vec<BienPrivadoId> = a
                .resultado
                .clone()
                .unwrap_or_default()
                .into_iter()
                .take(cupos)
                .collect();
            tx.inscripciones(a.id)
                .await?
                .into_iter()
                .map(|i| i.bien_privado_id)
                .filter(|b| !favorecidos.contains(b))
                .collect()
        }
        None => Vec::new(),
    };
    let inscritos: Vec<Inscrito> = tx
        .inscripciones(id)
        .await?
        .into_iter()
        .map(|i| Inscrito {
            bien_privado_id: i.bien_privado_id,
            discapacidad: i.discapacidad,
            no_favorecido_anterior: no_favorecidos.contains(&i.bien_privado_id),
        })
        .collect();
    let orden = ejecutar(&s.semilla, &s.criterios, &inscritos);
    let mut asignaciones = Vec::new();
    for (recurso, bien) in s.recursos.iter().zip(orden.iter()) {
        let a = AsignacionParqueadero {
            id: AsignacionParqueaderoId::nuevo(),
            recurso_id: *recurso,
            bien_privado_id: *bien,
            vehiculo_id: None,
            origen: OrigenAsignacionParqueadero::Sorteo,
            desde: s.vigencia_desde,
            hasta: Some(s.vigencia_hasta),
            sorteo_id: Some(id),
        };
        tx.insertar_asignacion_parqueadero(&a).await?;
        asignaciones.push(a);
    }
    tx.guardar_resultado_sorteo(id, &orden, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("inscritos", inscritos.len())?
        .con("asignados", asignaciones.len())?
        .con("semilla", s.semilla.as_str())?;
    tx.registrar(EventoAuditoria::ok(ACCION_EJECUTAR, "sorteo", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    s.estado = EstadoSorteo::Ejecutado;
    s.resultado = Some(orden);
    s.ejecutado_en = Some(ahora);
    Ok(ResultadoSorteo {
        sorteo: s,
        asignaciones,
    })
}

/// Sorteo con sus inscripciones. La semilla solo se muestra si ya se ejecutó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaSorteo {
    pub sorteo: Sorteo,
    pub semilla_revelada: Option<String>,
    pub inscripciones: Vec<InscripcionSorteo>,
}

/// Sorteos de la copropiedad (los ve cualquiera con `bienes_comunes.ver`: son públicos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.ver`.
pub async fn listar_sorteos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<VistaSorteo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    ctx.exigir(Permiso::BienesComunesVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let mut vistas = Vec::new();
    for s in tx.sorteos().await? {
        let inscripciones = tx.inscripciones(s.id).await?;
        vistas.push(VistaSorteo {
            semilla_revelada: (s.estado == EstadoSorteo::Ejecutado)
                .then(|| s.semilla.as_str().to_owned()),
            sorteo: s,
            inscripciones,
        });
    }
    Ok(vistas)
}

#[cfg(test)]
mod tests {
    use domain::{CanalReserva, Franja, Placa, sorteo};

    use super::*;
    use crate::casos_uso::{
        bienes_privados::{crear_bien_privado, tests::nuevo_bien},
        reservas::{
            SolicitudReserva, solicitar_reserva,
            tests::{Entorno, entorno, local, reloj, todos_los_dias},
        },
    };

    fn nuevo(codigo: &str, naturaleza: NaturalezaParqueadero) -> NuevoParqueadero {
        NuevoParqueadero {
            codigo: codigo.into(),
            tipo_vehiculo: TipoParqueadero::Carro,
            naturaleza,
            bien_privado_id: None,
            ubicacion: None,
            politica: None,
            horario: todos_los_dias(),
        }
    }

    async fn comunales(e: &Entorno, n: usize) -> Vec<RecursoId> {
        let mut ids = Vec::new();
        for i in 0..n {
            ids.push(
                crear_parqueadero(
                    &e.unidad,
                    &reloj(),
                    &e.ctx,
                    nuevo(&format!("C{i}"), NaturalezaParqueadero::Comunal),
                )
                .await
                .expect("parqueadero")
                .id,
            );
        }
        ids
    }

    #[tokio::test]
    async fn sorteo_reproducible_que_asigna_los_cupos() {
        let e = entorno().await;
        let recursos = comunales(&e, 2).await;
        let s = crear_sorteo(
            &e.unidad,
            &reloj(),
            &e.ctx,
            NuevoSorteo {
                nombre: "Primer semestre 2027".into(),
                recursos: recursos.clone(),
                vigencia_desde: local(30, 0),
                vigencia_hasta: local(30, 0) + chrono::Duration::days(180),
                criterios: CriteriosSorteo::default(),
            },
        )
        .await
        .expect("sorteo");
        assert_eq!(s.compromiso, s.semilla.compromiso());
        let mut bienes = vec![e.bien];
        for n in ["102", "103"] {
            bienes.push(
                crear_bien_privado(&e.unidad, &e.ctx, nuevo_bien(n, None))
                    .await
                    .expect("bien")
                    .id,
            );
        }
        inscribirse_sorteo(&e.unidad, &reloj(), &e.dueno, s.id, e.bien, false)
            .await
            .expect("el propietario se inscribe");
        assert!(matches!(
            inscribirse_sorteo(&e.unidad, &reloj(), &e.dueno, s.id, bienes[1], false).await,
            Err(AppError::SinPermiso(Permiso::SorteosGestionar))
        ));
        inscribirse_sorteo(&e.unidad, &reloj(), &e.ctx, s.id, bienes[1], true)
            .await
            .expect("2");
        inscribirse_sorteo(&e.unidad, &reloj(), &e.ctx, s.id, bienes[2], false)
            .await
            .expect("3");

        let r = ejecutar_sorteo(&e.unidad, &reloj(), &e.ctx, s.id)
            .await
            .expect("ejecuta");
        assert_eq!(r.asignaciones.len(), 2);
        let orden = r.sorteo.resultado.clone().expect("resultado");
        assert_eq!(orden[0], bienes[1], "la discapacidad tiene prioridad");
        let inscritos: Vec<sorteo::Inscrito> = bienes
            .iter()
            .map(|b| sorteo::Inscrito {
                bien_privado_id: *b,
                discapacidad: *b == bienes[1],
                no_favorecido_anterior: false,
            })
            .collect();
        assert_eq!(
            sorteo::ejecutar(&s.semilla, &s.criterios, &inscritos),
            orden,
            "cualquiera lo reproduce con la semilla revelada"
        );
        assert!(matches!(
            ejecutar_sorteo(&e.unidad, &reloj(), &e.ctx, s.id).await,
            Err(AppError::Conflicto(_))
        ));
        assert!(matches!(
            asignar_parqueadero(
                &e.unidad,
                &reloj(),
                &e.ctx,
                SolicitudAsignacionParqueadero {
                    recurso_id: recursos[0],
                    bien_privado_id: bienes[2],
                    vehiculo_id: None,
                    origen: OrigenAsignacionParqueadero::Manual,
                    desde: Some(local(30, 0) + chrono::Duration::days(10)),
                    hasta: None,
                }
            )
            .await,
            Err(AppError::Conflicto(_))
        ));
        let vistas = listar_sorteos(&e.unidad, &reloj(), &e.ctx)
            .await
            .expect("lista");
        assert_eq!(
            vistas[0].semilla_revelada.as_deref(),
            Some(s.semilla.as_str())
        );
    }

    #[tokio::test]
    async fn solo_parqueaderos_comunales_se_sortean_y_la_semilla_no_se_revela_antes() {
        let e = entorno().await;
        let visitantes = crear_parqueadero(
            &e.unidad,
            &reloj(),
            &e.ctx,
            nuevo("V1", NaturalezaParqueadero::Visitantes),
        )
        .await
        .expect("visitantes");
        let n = NuevoSorteo {
            nombre: "x".into(),
            recursos: vec![visitantes.id],
            vigencia_desde: local(30, 0),
            vigencia_hasta: local(30, 1),
            criterios: CriteriosSorteo::default(),
        };
        assert!(matches!(
            crear_sorteo(&e.unidad, &reloj(), &e.ctx, n).await,
            Err(AppError::Validacion(_))
        ));
        let recursos = comunales(&e, 1).await;
        crear_sorteo(
            &e.unidad,
            &reloj(),
            &e.ctx,
            NuevoSorteo {
                nombre: "abierto".into(),
                recursos,
                vigencia_desde: local(30, 0),
                vigencia_hasta: local(30, 1),
                criterios: CriteriosSorteo::default(),
            },
        )
        .await
        .expect("crea");
        let vistas = listar_sorteos(&e.unidad, &reloj(), &e.ctx)
            .await
            .expect("lista");
        assert_eq!(vistas[0].semilla_revelada, None);
    }

    #[tokio::test]
    async fn visitantes_se_reservan_con_placa() {
        let e = entorno().await;
        let p = crear_parqueadero(
            &e.unidad,
            &reloj(),
            &e.ctx,
            nuevo("V1", NaturalezaParqueadero::Visitantes),
        )
        .await
        .expect("visitantes");
        let mut s = SolicitudReserva {
            recurso_id: p.id,
            bien_privado_id: e.bien,
            franja: Franja::new(local(8, 9), local(8, 11)).expect("franja"),
            invitados: 0,
            idempotency_key: "v1".into(),
            canal: CanalReserva::Web,
            modalidad: None,
            motivo: None,
            placa_visitante: None,
        };
        assert!(
            solicitar_reserva(&e.unidad, &reloj(), &e.dueno, s.clone())
                .await
                .is_err()
        );
        s.placa_visitante = Some(Placa::new("ABC123").expect("placa"));
        let r = solicitar_reserva(&e.unidad, &reloj(), &e.dueno, s)
            .await
            .expect("reserva");
        assert_eq!(
            r.estado,
            domain::EstadoReserva::Aprobada,
            "confirmación inmediata"
        );
        let lista = listar_parqueaderos(&e.unidad, &reloj(), &e.ctx)
            .await
            .expect("lista");
        assert_eq!(lista.len(), 1);
    }
}
