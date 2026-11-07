//! Importación masiva de bienes privados, coeficientes y propietarios desde CSV/XLSX.
//!
//! Patrón reutilizable (plan 01 §7, construido en el plan 03):
//! 1. [`importar_bienes_privados`]: guarda la planilla como archivo (`casos_uso::archivos`,
//!    clasificación `personal`), la valida **fila por fila** contra el estado actual y registra
//!    la importación con sus errores. Devuelve la **vista previa**; no cambia nada más.
//! 2. [`confirmar_importacion`]: si no hubo errores, vuelve a leer y validar el archivo (el
//!    estado pudo cambiar) y aplica todo **en una transacción**: agrupaciones nuevas, bienes
//!    privados, personas, titulares (con su rol derivado) y un cuadro de coeficientes
//!    **borrador** con los coeficientes del archivo (activarlo sigue siendo una decisión aparte).
//!
//! Ni la auditoría ni los errores guardados repiten valores de las celdas.

use std::collections::{BTreeMap, BTreeSet};

use domain::{
    AgrupacionId, BienPrivadoId, ClasificacionArchivo, CodigoAgrupacion, CodigoBienPrivado,
    Coeficiente, ConjuntoTitulares, CuadroCoeficientesId, EstadoCuadro, EstadoImportacion,
    ImportacionId, NombreArchivo, Permiso, PlantillaNomenclatura, TipoAgrupacion, TipoArchivo,
    TipoBienPrivado, TipoCuadro, TipoImportacion, TipoPersona, TipoTitularidad, TitularNuevo,
    UsoBienPrivado,
    importacion::{Encabezados, ErrorFila, TitularImportado},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
        bienes_privados::ruta_de,
        titulares::{DatosPersona, RefPersona, insertar_conjunto, resolver_persona},
    },
    puertos::{
        AlmacenArchivos, LectorPlanillas, RegistroAuditoria, Reloj, RepositorioArchivos,
        RepositorioBienesPrivados, RepositorioCoeficientes, RepositorioCopropiedad,
        RepositorioImportaciones, RepositorioPersonas, RepositorioRoles, RepositorioTitulares,
        Transaccion, UnidadDeTrabajo,
        copropiedades::{
            Agrupacion, BienPrivado, CuadroCoeficientes, Importacion, ResumenImportacion,
        },
    },
};

pub const ACCION_IMPORTAR: AccionAuditoria =
    AccionAuditoria::new("importaciones.bienes_privados.validar");
pub const ACCION_CONFIRMAR: AccionAuditoria =
    AccionAuditoria::new("importaciones.bienes_privados.aplicar");

/// Filas de datos máximas por archivo.
pub const MAXIMO_FILAS: usize = 5_000;

/// Agrupación referida por una fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefAgrupacion {
    pub codigo: CodigoAgrupacion,
    /// `None` = no existe: se crea al aplicar (raíz, tipo edificio, nombre = código).
    pub id: Option<AgrupacionId>,
}

/// Bien privado que resultaría de la importación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienImportado {
    /// Primera fila del bien en la planilla.
    pub fila: u32,
    pub codigo: CodigoBienPrivado,
    pub agrupacion: Option<RefAgrupacion>,
    pub numero: Option<domain::NumeroBienPrivado>,
    pub tipo: TipoBienPrivado,
    pub area_m2: Option<Decimal>,
    pub coeficiente: Option<Coeficiente>,
    pub titulares: Vec<TitularImportado>,
}

/// Resultado de validar una planilla contra el estado actual.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Analisis {
    pub bienes: Vec<BienImportado>,
    pub errores: Vec<ErrorFila>,
    pub total_filas: u32,
    pub filas_validas: u32,
}

/// Valida las filas de la planilla (la primera, los encabezados). Función pura.
pub fn analizar(
    filas: &[Vec<String>],
    plantilla: &PlantillaNomenclatura,
    agrupaciones: &[Agrupacion],
    codigos_existentes: &BTreeSet<String>,
) -> Analisis {
    let mut a = Analisis::default();
    let Some((encabezado, datos)) = filas.split_first() else {
        a.errores
            .push(ErrorFila::new(1, None, "la planilla está vacía"));
        return a;
    };
    let encabezados = match Encabezados::detectar(encabezado) {
        Ok(e) => e,
        Err(errores) => {
            a.errores = errores;
            return a;
        }
    };
    let datos: Vec<(u32, &Vec<String>)> = datos
        .iter()
        .enumerate()
        .filter(|(_, f)| f.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, f)| (u32::try_from(i + 2).unwrap_or(u32::MAX), f))
        .collect();
    a.total_filas = u32::try_from(datos.len()).unwrap_or(u32::MAX);
    if datos.len() > MAXIMO_FILAS {
        a.errores.push(ErrorFila::new(
            1,
            None,
            format!("máximo {MAXIMO_FILAS} filas por archivo"),
        ));
        return a;
    }
    let mut con_error: BTreeSet<u32> = BTreeSet::new();
    let mut por_codigo: BTreeMap<String, usize> = BTreeMap::new();
    for (n, celdas) in datos {
        let fila = match encabezados.fila(n, celdas) {
            Ok(f) => f,
            Err(errores) => {
                con_error.insert(n);
                a.errores.extend(errores);
                continue;
            }
        };
        let mut error = |columna: Option<&str>, mensaje: &str| {
            con_error.insert(n);
            a.errores.push(ErrorFila::new(n, columna, mensaje));
        };
        let (agrupacion, ruta) = match &fila.agrupacion {
            None => (None, Vec::new()),
            Some(cod) => {
                let iguales: Vec<&Agrupacion> =
                    agrupaciones.iter().filter(|g| g.codigo == *cod).collect();
                match iguales.as_slice() {
                    [] => (
                        Some(RefAgrupacion {
                            codigo: cod.clone(),
                            id: None,
                        }),
                        vec![cod.clone()],
                    ),
                    [g] => match ruta_de(agrupaciones, g.id) {
                        Ok(r) => (
                            Some(RefAgrupacion {
                                codigo: cod.clone(),
                                id: Some(g.id),
                            }),
                            r.into_iter().map(|x| x.codigo.clone()).collect(),
                        ),
                        Err(_) => {
                            error(Some("agrupacion"), "agrupación inconsistente");
                            continue;
                        }
                    },
                    _ => {
                        error(
                            Some("agrupacion"),
                            "hay varias agrupaciones con ese código; use el código del bien",
                        );
                        continue;
                    }
                }
            }
        };
        let codigo = match (&fila.codigo, &fila.numero) {
            (Some(c), _) => c.clone(),
            (None, Some(numero)) => match plantilla.codigo(&ruta, numero) {
                Ok(c) => c,
                Err(_) => {
                    error(Some("numero"), "no se pudo calcular el código del bien");
                    continue;
                }
            },
            (None, None) => {
                error(
                    Some("numero"),
                    "falta el número (o el código) del bien privado",
                );
                continue;
            }
        };
        if let Some(&i) = por_codigo.get(codigo.as_str()) {
            match fila.titular {
                Some(t) => a.bienes[i].titulares.push(t),
                None => error(
                    None,
                    "fila repetida de un bien privado: las filas siguientes solo agregan propietarios",
                ),
            }
            continue;
        }
        if codigos_existentes.contains(codigo.as_str()) {
            error(Some("codigo"), "ya existe un bien privado con ese código");
            continue;
        }
        por_codigo.insert(codigo.as_str().to_owned(), a.bienes.len());
        a.bienes.push(BienImportado {
            fila: n,
            codigo,
            agrupacion,
            numero: fila.numero,
            tipo: fila.tipo,
            area_m2: fila.area_m2,
            coeficiente: fila.coeficiente,
            titulares: fila.titular.into_iter().collect(),
        });
    }
    // Ids provisionales por documento solo para validar los conjuntos de titulares.
    let mut provisionales: BTreeMap<String, domain::PersonaId> = BTreeMap::new();
    for b in &a.bienes {
        for t in &b.titulares {
            let n = provisionales.len() as u128 + 1;
            provisionales
                .entry(t.documento.as_str().to_owned())
                .or_insert_with(|| domain::PersonaId::from_uuid(uuid::Uuid::from_u128(n)));
        }
    }
    for b in &a.bienes {
        if b.titulares.is_empty() {
            continue;
        }
        if let Err(e) = conjunto_de(b, |t| provisionales[t.documento.as_str()]) {
            con_error.insert(b.fila);
            let mensaje = match e {
                AppError::Dominio(domain::DominioError::TitularesInvalidos(m)) => m.to_owned(),
                _ => "propietarios inválidos".to_owned(),
            };
            a.errores
                .push(ErrorFila::new(b.fila, Some("porcentaje"), mensaje));
        }
    }
    a.errores.sort_by_key(|e| e.fila);
    a.filas_validas = a
        .total_filas
        .saturating_sub(u32::try_from(con_error.len()).unwrap_or(u32::MAX));
    a
}

/// Conjunto de titulares de un bien importado; el primero representa al bien.
fn conjunto_de(
    b: &BienImportado,
    id_de: impl Fn(&TitularImportado) -> domain::PersonaId,
) -> AppResult<ConjuntoTitulares> {
    Ok(ConjuntoTitulares::new(
        b.titulares
            .iter()
            .enumerate()
            .map(|(i, t)| TitularNuevo {
                persona_id: id_de(t),
                tipo: TipoTitularidad::Propietario,
                porcentaje: t.porcentaje,
                es_representante: i == 0,
            })
            .collect(),
    )?)
}

/// Fila de la vista previa (sin datos personales).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaVistaPrevia {
    pub fila: u32,
    pub codigo: CodigoBienPrivado,
    pub tipo: TipoBienPrivado,
    /// Código de la agrupación y si se crearía.
    pub agrupacion: Option<(CodigoAgrupacion, bool)>,
    pub coeficiente: Option<Coeficiente>,
    pub titulares: u32,
}

/// Importación registrada y su vista previa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaPrevia {
    pub importacion: Importacion,
    pub filas: Vec<FilaVistaPrevia>,
}

async fn analizar_en<T>(tx: &mut T, filas: &[Vec<String>]) -> AppResult<Analisis>
where
    T: RepositorioCopropiedad + RepositorioBienesPrivados,
{
    let plantilla = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?
        .plantilla_codigo;
    let agrupaciones = tx.agrupaciones().await?;
    let codigos: BTreeSet<String> = tx
        .todos_los_bienes()
        .await?
        .into_iter()
        .map(|b| b.codigo.as_str().to_owned())
        .collect();
    Ok(analizar(filas, &plantilla, &agrupaciones, &codigos))
}

fn resumen_previo(a: &Analisis) -> ResumenImportacion {
    let nuevas: BTreeSet<&str> = a
        .bienes
        .iter()
        .filter_map(|b| b.agrupacion.as_ref())
        .filter(|g| g.id.is_none())
        .map(|g| g.codigo.as_str())
        .collect();
    let contar = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    ResumenImportacion {
        bienes_privados: contar(a.bienes.len()),
        agrupaciones_nuevas: contar(nuevas.len()),
        titulares: contar(a.bienes.iter().map(|b| b.titulares.len()).sum()),
        personas_nuevas: 0,
        coeficientes: contar(a.bienes.iter().filter(|b| b.coeficiente.is_some()).count()),
        cuadro_borrador_id: None,
    }
}

/// Sube y valida una planilla de bienes privados. No aplica nada: devuelve la vista previa.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar`.
/// - [`AppError::Dominio`] si el archivo está vacío, no es CSV/XLSX o es muy grande.
/// - [`AppError::Validacion`] si la planilla no se puede leer.
pub async fn importar_bienes_privados<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<VistaPrevia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos
        + RepositorioImportaciones
        + RepositorioBienesPrivados
        + RepositorioCopropiedad
        + RegistroAuditoria,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    let tipo = TipoArchivo::validar_planilla(&contenido)?;
    let filas = lector.leer(tipo, &contenido)?;
    let subido = ArchivoSubido {
        nombre_original: nombre,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        let mut tx = unidad.iniciar(ctx).await?;
        let analisis = analizar_en(&mut tx, &filas).await?;
        registrar_en(&mut tx, &archivo).await?;
        let importacion = Importacion {
            id: ImportacionId::nuevo(),
            tipo: TipoImportacion::BienesPrivados,
            archivo_id: archivo.id,
            estado: if analisis.errores.is_empty() {
                EstadoImportacion::Validada
            } else {
                EstadoImportacion::ConErrores
            },
            total_filas: analisis.total_filas,
            filas_validas: analisis.filas_validas,
            errores: analisis.errores.clone(),
            resumen: resumen_previo(&analisis),
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
            .bienes
            .iter()
            .map(|b| FilaVistaPrevia {
                fila: b.fila,
                codigo: b.codigo.clone(),
                tipo: b.tipo,
                agrupacion: b
                    .agrupacion
                    .as_ref()
                    .map(|g| (g.codigo.clone(), g.id.is_none())),
                coeficiente: b.coeficiente,
                titulares: u32::try_from(b.titulares.len()).unwrap_or(u32::MAX),
            })
            .collect();
        Ok(VistaPrevia { importacion, filas })
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Una importación con sus errores y su resumen.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `importaciones.ejecutar`; [`AppError::NoEncontrado`].
pub async fn ver_importacion<U>(
    unidad: &U,
    ctx: &Contexto,
    id: ImportacionId,
) -> AppResult<Importacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioImportaciones,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    unidad
        .iniciar(ctx)
        .await?
        .importacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("importación"))
}

/// Aplica una importación validada, en una sola transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar`; [`AppError::NoEncontrado`].
/// - [`AppError::Conflicto`] si ya se aplicó, tenía errores o los datos cambiaron desde la
///   vista previa (hay que importar de nuevo).
pub async fn confirmar_importacion<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ImportacionId,
) -> AppResult<Importacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos
        + RepositorioImportaciones
        + RepositorioBienesPrivados
        + RepositorioCopropiedad
        + RepositorioCoeficientes
        + RepositorioTitulares
        + RepositorioPersonas
        + RepositorioRoles
        + RegistroAuditoria,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut importacion = tx
        .importacion(id)
        .await?
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
    let analisis = analizar_en(&mut tx, &filas).await?;
    if !analisis.errores.is_empty() {
        return Err(AppError::Conflicto(
            "los datos cambiaron desde la vista previa: importe el archivo de nuevo".into(),
        ));
    }
    let resumen = aplicar(&mut tx, reloj, ctx, &analisis).await?;
    let ahora = reloj.ahora();
    tx.marcar_aplicada(id, &resumen, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bienes_privados", resumen.bienes_privados)?
        .con("titulares", resumen.titulares)?
        .con("coeficientes", resumen.coeficientes)?;
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

async fn aplicar<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    analisis: &Analisis,
) -> AppResult<ResumenImportacion>
where
    T: RepositorioBienesPrivados
        + RepositorioCoeficientes
        + RepositorioTitulares
        + RepositorioPersonas
        + RepositorioRoles
        + RegistroAuditoria,
{
    let mut resumen = resumen_previo(analisis);
    let mut nuevas: BTreeMap<String, AgrupacionId> = BTreeMap::new();
    let mut coeficientes: BTreeMap<BienPrivadoId, Coeficiente> = BTreeMap::new();
    for b in &analisis.bienes {
        let agrupacion_id = match &b.agrupacion {
            None => None,
            Some(RefAgrupacion { id: Some(id), .. }) => Some(*id),
            Some(RefAgrupacion {
                codigo: cod,
                id: None,
            }) => match nuevas.get(cod.as_str()) {
                Some(id) => Some(*id),
                None => {
                    let g = Agrupacion {
                        id: AgrupacionId::nuevo(),
                        tipo: TipoAgrupacion::Edificio,
                        nombre: cod.as_str().to_owned(),
                        codigo: cod.clone(),
                        padre_id: None,
                        orden: 0,
                    };
                    tx.insertar_agrupacion(&g).await?;
                    nuevas.insert(cod.as_str().to_owned(), g.id);
                    Some(g.id)
                }
            },
        };
        let bien = BienPrivado {
            id: BienPrivadoId::nuevo(),
            codigo: b.codigo.clone(),
            tipo: b.tipo,
            agrupacion_id,
            numero: b.numero.clone(),
            piso: None,
            area_privada_m2: b.area_m2,
            matricula_inmobiliaria: None,
            cedula_catastral: None,
            uso: UsoBienPrivado::Residencial,
            activo: true,
            bien_principal_id: None,
            coeficiente: None,
        };
        tx.insertar_bien(&bien).await?;
        if let Some(c) = b.coeficiente {
            coeficientes.insert(bien.id, c);
        }
        if b.titulares.is_empty() {
            continue;
        }
        let mut personas = Vec::new();
        for t in &b.titulares {
            let (p, nueva) = resolver_persona(
                tx,
                RefPersona::Nueva(DatosPersona {
                    tipo: TipoPersona::Natural,
                    nombre: t.nombre.clone(),
                    documento: t.documento.clone(),
                    telefono: t.telefono.clone(),
                    correo: t.correo.clone(),
                    cuenta_id: None,
                    tipo_documento: None,
                }),
            )
            .await?;
            resumen.personas_nuevas += u32::from(nueva);
            personas.push(p);
        }
        let conjunto = conjunto_de(b, |t| {
            personas
                .iter()
                .find(|p| p.documento.as_ref() == Some(&t.documento))
                .map(|p| p.id)
                .expect("cada titular importado tiene su persona resuelta")
        })?;
        insertar_conjunto(tx, reloj, ctx, bien.id, &personas, &conjunto, None).await?;
    }
    if !coeficientes.is_empty() {
        let cuadro = CuadroCoeficientes {
            id: CuadroCoeficientesId::nuevo(),
            version: tx.ultima_version().await? + 1,
            tipo: TipoCuadro::Copropiedad,
            estado: EstadoCuadro::Borrador,
            descripcion: Some("Coeficientes de una importación masiva".into()),
            soporte_archivo_id: None,
            vigente_desde: None,
            aprobado_por: None,
            creado_en: reloj.ahora(),
        };
        tx.insertar_cuadro(&cuadro).await?;
        // Los bienes que no venían en el archivo conservan su coeficiente vigente.
        let mut filas = match tx.cuadro_vigente().await? {
            Some(v) => tx.coeficientes(v.id).await?,
            None => BTreeMap::new(),
        };
        filas.extend(coeficientes);
        tx.reemplazar_coeficientes(cuadro.id, &filas).await?;
        resumen.cuadro_borrador_id = Some(cuadro.id);
    }
    Ok(resumen)
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn planilla(filas: &[&[&str]]) -> Vec<Vec<String>> {
        filas
            .iter()
            .map(|f| f.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    fn analizar_simple(filas: &[&[&str]]) -> Analisis {
        analizar(
            &planilla(filas),
            &PlantillaNomenclatura::new("{agrupacion}{numero}").expect("válida"),
            &[],
            &BTreeSet::from(["3999".to_owned()]),
        )
    }

    const ENCABEZADO: &[&str] = &[
        "agrupacion",
        "numero",
        "tipo",
        "coeficiente",
        "propietario",
        "documento",
        "porcentaje",
    ];

    #[test]
    fn agrupa_copropietarios_y_calcula_codigos() {
        let a = analizar_simple(&[
            ENCABEZADO,
            &["3", "506", "", "0,6 %", "Ana", "111", "50"],
            &["3", "506", "", "", "Luis", "222", "50"],
            &["", "", "", "", "", "", ""],
            &["3", "P1", "parqueadero", "0,1 %", "", "", ""],
        ]);
        assert!(a.errores.is_empty(), "{:?}", a.errores);
        assert_eq!(a.total_filas, 3);
        assert_eq!(a.bienes.len(), 2);
        assert_eq!(a.bienes[0].codigo.as_str(), "3506");
        assert_eq!(a.bienes[0].titulares.len(), 2);
        assert_eq!(
            a.bienes[0].coeficiente.map(|c| c.valor()),
            Some(dec!(0.006))
        );
        assert_eq!(a.bienes[1].tipo, TipoBienPrivado::Parqueadero);
        assert_eq!(
            a.bienes[0].agrupacion,
            Some(RefAgrupacion {
                codigo: CodigoAgrupacion::new("3").expect("válido"),
                id: None
            })
        );
    }

    #[test]
    fn reporta_errores_por_fila() {
        let a = analizar_simple(&[
            ENCABEZADO,
            &["3", "999", "", "", "", "", ""],
            &["3", "101", "", "dos", "", "", ""],
            &["3", "102", "", "", "Ana", "111", "60"],
            &["3", "102", "", "", "Luis", "222", "30"],
            &["3", "103", "", "", "", "", ""],
            &["3", "103", "", "", "", "", ""],
        ]);
        let filas: Vec<_> = a.errores.iter().map(|e| e.fila).collect();
        assert_eq!(filas, vec![2, 3, 4, 7], "{:?}", a.errores);
        assert_eq!(a.errores[0].columna.as_deref(), Some("codigo"));
        assert_eq!(a.filas_validas, 2);
    }

    #[test]
    fn planilla_vacia_o_sin_encabezados_validos() {
        assert_eq!(
            analizar_simple(&[]).errores[0].mensaje,
            "la planilla está vacía"
        );
        assert_eq!(analizar_simple(&[&["nombre", "valor"]]).errores.len(), 1);
    }
}
