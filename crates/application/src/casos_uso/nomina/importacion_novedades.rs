//! Carga masiva de novedades de nómina (plan 16) con el flujo de las importaciones: planilla →
//! validación fila por fila → vista previa → confirmar → aplicar en **una** transacción. La
//! planilla la lee [`domain::importacion::novedades`]; aquí se busca el empleado por su código y
//! el contrato que cubre las fechas de cada novedad.

use domain::{
    ClasificacionArchivo, ContratoLaboralId, EstadoImportacion, ImportacionId, NombreArchivo,
    Permiso, TipoArchivo, TipoImportacion,
    importacion::{
        ErrorFila,
        novedades::{NovedadImportada, leer_planilla_novedades},
    },
    nomina::liquidacion::TipoNovedad,
};

use super::{
    abrir_nomina,
    importacion::TxImportacionEmpleados,
    novedades::{NuevaNovedad, registrar_en},
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en as subir},
    puertos::{
        AlmacenArchivos, LectorPlanillas, RegistroAuditoria as _, Reloj, RepositorioArchivos as _,
        Transaccion, UnidadDeTrabajo,
        importaciones::{Importacion, RepositorioImportaciones as _, ResumenImportacion},
    },
};

const ACCION_IMPORTAR: AccionAuditoria = AccionAuditoria::new("importaciones.novedades.validar");
const ACCION_CONFIRMAR: AccionAuditoria = AccionAuditoria::new("importaciones.novedades.aplicar");

/// Fila válida de la vista previa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaNovedadPrevia {
    pub fila: u32,
    /// Código del empleado.
    pub codigo: String,
    pub tipo: TipoNovedad,
}

/// Importación de novedades y su vista previa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaPreviaNovedades {
    pub importacion: Importacion,
    pub filas: Vec<FilaNovedadPrevia>,
}

struct Analisis {
    novedades: Vec<(NovedadImportada, ContratoLaboralId)>,
    errores: Vec<ErrorFila>,
    total_filas: u32,
}

fn contar(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

async fn analizar<T: TxImportacionEmpleados>(
    tx: &mut T,
    filas: &[Vec<String>],
) -> AppResult<Analisis> {
    let planilla = leer_planilla_novedades(filas);
    let mut a = Analisis {
        novedades: Vec::new(),
        errores: planilla.errores,
        total_filas: planilla.total_filas,
    };
    for n in planilla.novedades {
        let Some(empleado) = tx.empleado_por_codigo(&n.codigo).await? else {
            a.errores.push(ErrorFila::new(
                n.fila,
                Some("codigo"),
                "no hay un empleado con ese código",
            ));
            continue;
        };
        let contrato = tx.contratos(empleado.id).await?.into_iter().find(|c| {
            let fin = c.fecha_terminacion.or(c.fecha_fin);
            n.novedad.desde >= c.fecha_inicio && fin.is_none_or(|f| n.novedad.hasta <= f)
        });
        match contrato {
            Some(c) => a.novedades.push((n, c.id)),
            None => a.errores.push(ErrorFila::new(
                n.fila,
                Some("desde"),
                "la novedad cae fuera de los contratos del empleado",
            )),
        }
    }
    a.errores.sort_by_key(|e| e.fila);
    Ok(a)
}

/// Sube y valida una planilla de novedades. No registra nada: devuelve la vista previa.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o `nomina.novedades.registrar`;
///   [`AppError::ModuloNoHabilitado`] sin nómina.
/// - [`AppError::Dominio`] si el archivo está vacío, no es CSV/XLSX o es muy grande.
/// - [`AppError::Validacion`] si la planilla no se puede leer.
pub async fn importar_novedades<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<VistaPreviaNovedades>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxImportacionEmpleados,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    let tipo = TipoArchivo::validar_planilla(&contenido)?;
    let filas = lector.leer(tipo, &contenido)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaNovedadesRegistrar).await?;
    let subido = ArchivoSubido {
        nombre_original: nombre,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        let analisis = analizar(&mut tx, &filas).await?;
        subir(&mut tx, &archivo).await?;
        let importacion = Importacion {
            id: ImportacionId::nuevo(),
            tipo: TipoImportacion::NovedadesNomina,
            archivo_id: archivo.id,
            estado: if analisis.errores.is_empty() {
                EstadoImportacion::Validada
            } else {
                EstadoImportacion::ConErrores
            },
            total_filas: analisis.total_filas,
            filas_validas: contar(analisis.novedades.len()),
            errores: analisis.errores.clone(),
            resumen: ResumenImportacion {
                novedades: contar(analisis.novedades.len()),
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
            .novedades
            .iter()
            .map(|(n, _)| FilaNovedadPrevia {
                fila: n.fila,
                codigo: n.codigo.as_str().to_owned(),
                tipo: n.novedad.tipo,
            })
            .collect();
        Ok(VistaPreviaNovedades { importacion, filas })
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Registra las novedades de una importación validada en una sola transacción (si una cae en un
/// periodo aprobado, no se registra ninguna y el mensaje dice la fila).
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o `nomina.novedades.registrar`.
/// - [`AppError::NoEncontrado`] si no existe o no es de novedades.
/// - [`AppError::Conflicto`] si ya se aplicó, tenía errores, los datos cambiaron o una fila choca.
pub async fn confirmar_importacion_novedades<U, A, L>(
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
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaNovedadesRegistrar).await?;
    let mut importacion = tx
        .importacion(id)
        .await?
        .filter(|i| i.tipo == TipoImportacion::NovedadesNomina)
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
    let total = contar(analisis.novedades.len());
    for (n, contrato) in analisis.novedades {
        let medida = n.novedad.tipo.medida();
        let nueva = NuevaNovedad {
            tipo: n.novedad.tipo,
            desde: n.novedad.desde,
            hasta: (medida == domain::nomina::liquidacion::Medida::Dias).then_some(n.novedad.hasta),
            cantidad: (medida != domain::nomina::liquidacion::Medida::Dias)
                .then_some(n.novedad.cantidad),
            descripcion: n.descripcion,
            soporte_archivo_id: None,
        };
        registrar_en(&mut tx, reloj, ctx, contrato, nueva)
            .await
            .map_err(|e| match e {
                AppError::Infraestructura(_) => e,
                otro => AppError::Conflicto(format!("fila {}: {otro}", n.fila)),
            })?;
    }
    let resumen = ResumenImportacion {
        novedades: total,
        ..ResumenImportacion::default()
    };
    let ahora = reloj.ahora();
    tx.marcar_aplicada(id, &resumen, ahora).await?;
    let detalle = DetalleAuditoria::vacio().con("novedades", total)?;
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
