//! Carga masiva de empleados (plan 16 fase 1) con el flujo de las importaciones: planilla →
//! validación fila por fila → vista previa → confirmar → aplicar en **una** transacción (si una
//! fila falla al aplicar, no se crea ninguno y el mensaje dice cuál). La planilla la lee
//! [`domain::importacion::empleados`]; aquí se resuelven las administradoras por su código de
//! la PILA y cada contrato se valida contra el catálogo laboral vigente en su inicio.

use std::collections::BTreeMap;

use domain::{
    AdministradoraId, ClasificacionArchivo, EstadoImportacion, ImportacionId, NombreArchivo,
    Permiso, TipoArchivo, TipoImportacion,
    importacion::{
        ErrorFila,
        empleados::{EmpleadoImportado, leer_planilla_empleados},
    },
    nomina::TipoAdministradora,
};

use super::{
    TxNomina, abrir_nomina,
    empleados::{
        CondicionesVersion, NuevaPersonaEmpleado, NuevoContrato, NuevoEmpleado, PersonaEmpleado,
        crear_empleado_en, validar_nuevo_contrato,
    },
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    puertos::{
        AlmacenArchivos, LectorPlanillas, RegistroAuditoria as _, Reloj, RepositorioArchivos,
        Transaccion, UnidadDeTrabajo,
        importaciones::{Importacion, RepositorioImportaciones, ResumenImportacion},
        nomina::CuentaNomina,
    },
};

const ACCION_IMPORTAR: AccionAuditoria = AccionAuditoria::new("importaciones.empleados.validar");
const ACCION_CONFIRMAR: AccionAuditoria = AccionAuditoria::new("importaciones.empleados.aplicar");

/// Lo que usa una transacción de la importación de empleados.
pub trait TxImportacionEmpleados:
    TxNomina + RepositorioArchivos + RepositorioImportaciones
{
}

impl<T> TxImportacionEmpleados for T where
    T: TxNomina + RepositorioArchivos + RepositorioImportaciones
{
}

/// Fila válida de la vista previa (sin datos personales: la fila, el código y el contrato).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaEmpleadoPrevia {
    pub fila: u32,
    pub codigo: String,
    pub tipo_contrato: domain::nomina::TipoContratoLaboral,
}

/// Importación de empleados y su vista previa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaPreviaEmpleados {
    pub importacion: Importacion,
    pub filas: Vec<FilaEmpleadoPrevia>,
}

/// Planilla leída y validada contra el catálogo y los empleados de la entidad.
struct Analisis {
    empleados: Vec<(u32, NuevoEmpleado)>,
    errores: Vec<ErrorFila>,
    total_filas: u32,
}

fn contar(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Convierte la fila leída en el alta del empleado, resolviendo las administradoras.
fn alta_de(
    e: EmpleadoImportado,
    codigos: &BTreeMap<(TipoAdministradora, String), AdministradoraId>,
) -> Result<NuevoEmpleado, ErrorFila> {
    let fila = e.fila;
    let buscar = |tipo: TipoAdministradora, codigo: Option<&String>| match codigo {
        None => Ok(None),
        Some(c) => codigos
            .get(&(tipo, c.clone()))
            .copied()
            .map(Some)
            .ok_or_else(|| {
                ErrorFila::new(
                    fila,
                    Some(tipo.as_str()),
                    "no hay una administradora activa con ese código",
                )
            }),
    };
    let eps = buscar(TipoAdministradora::Eps, Some(&e.eps))?
        .ok_or_else(|| ErrorFila::new(fila, Some("eps"), "falta el dato"))?;
    let condiciones = CondicionesVersion {
        cargo: e.cargo,
        salario: e.salario,
        tipo_salario: e.tipo_salario,
        etapa_aprendizaje: e.etapa_aprendizaje,
        jornada: e.jornada,
        horas_semanales: e.horas_semanales,
        periodicidad: e.periodicidad,
        eps_id: eps,
        afp_id: buscar(TipoAdministradora::Afp, e.afp.as_ref())?,
        cesantias_id: buscar(TipoAdministradora::Cesantias, e.cesantias.as_ref())?,
        ccf_id: buscar(TipoAdministradora::Ccf, e.ccf.as_ref())?,
        arl_id: buscar(TipoAdministradora::Arl, e.arl.as_ref())?,
        clase_riesgo: e.clase_riesgo,
        pensionado: e.pensionado,
        medio_pago: e.medio_pago,
        cuenta: e.cuenta.map(|c| CuentaNomina {
            banco: c.banco,
            tipo: c.tipo,
            numero: c.numero,
        }),
    };
    Ok(NuevoEmpleado {
        codigo: e.codigo,
        persona: PersonaEmpleado::Nueva(NuevaPersonaEmpleado {
            nombre: e.nombre,
            tipo_documento: e.tipo_documento,
            documento: e.documento,
            correo: e.correo,
            telefono: e.telefono,
        }),
        contrato: NuevoContrato {
            tipo: e.tipo_contrato,
            fecha_inicio: e.fecha_inicio,
            fecha_fin: e.fecha_fin,
            descripcion_obra: e.descripcion_obra,
            condiciones,
        },
    })
}

/// Mensaje de un error de una fila sin el valor de la celda (los de la aplicación nombran
/// reglas y columnas, nunca datos).
fn mensaje(e: &AppError) -> String {
    match e {
        AppError::Validacion(m) | AppError::Conflicto(m) => m.clone(),
        AppError::Dominio(d) => d.to_string(),
        otro => otro.to_string(),
    }
}

async fn analizar<T: TxNomina>(tx: &mut T, filas: &[Vec<String>]) -> AppResult<Analisis> {
    let planilla = leer_planilla_empleados(filas);
    let codigos: BTreeMap<_, _> = tx
        .administradoras(true)
        .await?
        .into_iter()
        .map(|a| ((a.tipo, a.codigo), a.id))
        .collect();
    let mut a = Analisis {
        empleados: Vec::new(),
        errores: planilla.errores,
        total_filas: planilla.total_filas,
    };
    for e in planilla.empleados {
        let fila = e.fila;
        let alta = match alta_de(e, &codigos) {
            Ok(alta) => alta,
            Err(error) => {
                a.errores.push(error);
                continue;
            }
        };
        if let PersonaEmpleado::Nueva(p) = &alta.persona
            && let Some(persona) = tx.persona_por_documento(&p.documento).await?
            && tx.empleado_de_persona(persona.id).await?.is_some()
        {
            a.errores.push(ErrorFila::new(
                fila,
                Some("documento"),
                "la persona ya es empleada de la entidad",
            ));
            continue;
        }
        match validar_nuevo_contrato(tx, &alta.contrato).await {
            Ok(_) => a.empleados.push((fila, alta)),
            Err(e @ (AppError::Validacion(_) | AppError::Dominio(_))) => {
                a.errores.push(ErrorFila::new(fila, None, mensaje(&e)));
            }
            Err(otro) => return Err(otro),
        }
    }
    a.errores.sort_by_key(|e| e.fila);
    Ok(a)
}

/// Sube y valida una planilla de empleados. No crea nada: devuelve la vista previa con los
/// errores por fila.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o sin `nomina.empleados.gestionar`;
///   [`AppError::ModuloNoHabilitado`] sin nómina.
/// - [`AppError::Dominio`] si el archivo está vacío, no es CSV/XLSX o es muy grande.
/// - [`AppError::Validacion`] si la planilla no se puede leer.
pub async fn importar_empleados<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<VistaPreviaEmpleados>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxImportacionEmpleados,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    let tipo = TipoArchivo::validar_planilla(&contenido)?;
    let filas = lector.leer(tipo, &contenido)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let subido = ArchivoSubido {
        nombre_original: nombre,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        let analisis = analizar(&mut tx, &filas).await?;
        registrar_en(&mut tx, &archivo).await?;
        let importacion = Importacion {
            id: ImportacionId::nuevo(),
            tipo: TipoImportacion::Empleados,
            archivo_id: archivo.id,
            estado: if analisis.errores.is_empty() {
                EstadoImportacion::Validada
            } else {
                EstadoImportacion::ConErrores
            },
            total_filas: analisis.total_filas,
            filas_validas: contar(analisis.empleados.len()),
            errores: analisis.errores.clone(),
            resumen: ResumenImportacion {
                empleados: contar(analisis.empleados.len()),
                ..ResumenImportacion::default()
            },
            creada_por: ctx.actor_id,
            creada_en: reloj.ahora(),
            aplicada_en: None,
        };
        tx.insertar_importacion(&importacion).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("filas", importacion.total_filas)?
            .con("errores", importacion.errores.len())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_IMPORTAR, "importacion", importacion.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        let filas = analisis
            .empleados
            .iter()
            .map(|(fila, e)| FilaEmpleadoPrevia {
                fila: *fila,
                codigo: e.codigo.as_str().to_owned(),
                tipo_contrato: e.contrato.tipo,
            })
            .collect();
        Ok(VistaPreviaEmpleados { importacion, filas })
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Crea los empleados de una importación validada, en una sola transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o `nomina.empleados.gestionar`;
///   [`AppError::ModuloNoHabilitado`] sin nómina.
/// - [`AppError::NoEncontrado`] si no existe o no es de empleados.
/// - [`AppError::Conflicto`] si ya se aplicó, tenía errores, los datos cambiaron desde la vista
///   previa o una fila choca con lo que hay (el mensaje dice cuál).
pub async fn confirmar_importacion_empleados<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ImportacionId,
) -> AppResult<Importacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxImportacionEmpleados,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let mut importacion = tx
        .importacion(id)
        .await?
        .filter(|i| i.tipo == TipoImportacion::Empleados)
        .ok_or(AppError::NoEncontrado("importación"))?;
    match importacion.estado {
        EstadoImportacion::Validada => {}
        EstadoImportacion::Aplicada => {
            return Err(AppError::Conflicto("la importación ya se aplicó".into()));
        }
        EstadoImportacion::ConErrores => {
            return Err(AppError::Conflicto(
                "la importación tiene errores: corrija el archivo e impórtelo de nuevo".into(),
            ));
        }
    }
    let archivo = tx
        .buscar(importacion.archivo_id)
        .await?
        .ok_or(AppError::NoEncontrado("archivo"))?;
    let contenido = almacen.obtener(&archivo.clave).await?;
    let filas = lector.leer(archivo.tipo, &contenido)?;
    let analisis = analizar(&mut tx, &filas).await?;
    if !analisis.errores.is_empty() {
        return Err(AppError::Conflicto(
            "los datos cambiaron desde la vista previa: importe el archivo de nuevo".into(),
        ));
    }
    let mut personas_nuevas = 0;
    let total = contar(analisis.empleados.len());
    for (fila, alta) in analisis.empleados {
        let (_, nueva) =
            crear_empleado_en(&mut tx, reloj, ctx, alta)
                .await
                .map_err(|e| match e {
                    AppError::Conflicto(_) | AppError::Validacion(_) | AppError::Dominio(_) => {
                        AppError::Conflicto(format!("fila {fila}: {}", mensaje(&e)))
                    }
                    otro => otro,
                })?;
        personas_nuevas += u32::from(nueva);
    }
    let resumen = ResumenImportacion {
        empleados: total,
        personas_nuevas,
        ..ResumenImportacion::default()
    };
    let ahora = reloj.ahora();
    tx.marcar_aplicada(id, &resumen, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("empleados", resumen.empleados)?
        .con("personas_nuevas", personas_nuevas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONFIRMAR, "importacion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    importacion.estado = EstadoImportacion::Aplicada;
    importacion.resumen = resumen;
    importacion.aplicada_en = Some(ahora);
    Ok(importacion)
}
