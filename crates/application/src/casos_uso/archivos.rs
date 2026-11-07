//! Casos de uso de archivos: subir, generar el enlace de descarga y migrar entre almacenes.
//!
//! Permisos: `archivos.subir` para subir; `archivos.ver` para pedir el enlace de un archivo
//! público o interno, y además `archivos.personales.ver` si es personal o sensible.

use std::time::Duration;

use domain::{ArchivoId, ClasificacionArchivo, ClaveArchivo, NombreArchivo, Permiso, TipoArchivo};
use sha2::{Digest, Sha256};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        AlmacenArchivos, RegistroAuditoria, Reloj, RepositorioArchivos, UnidadDeTrabajo,
        archivos::{Archivo, DescargaArchivo},
    },
};

pub const ACCION_SUBIR: AccionAuditoria = AccionAuditoria::new("archivos.archivo.subir");
pub const ACCION_MIGRAR: AccionAuditoria = AccionAuditoria::new("archivos.archivo.migrar");

/// Vigencia por defecto de un enlace de descarga: suficiente para abrirlo, corta para que un
/// enlace reenviado deje de servir pronto.
pub const VIGENCIA_ENLACE: Duration = Duration::from_secs(5 * 60);

/// Archivo que llega para guardarse.
#[derive(Debug, Clone)]
pub struct ArchivoSubido {
    pub nombre_original: NombreArchivo,
    pub clasificacion: ClasificacionArchivo,
    pub contenido: Vec<u8>,
}

/// SHA-256 en hexadecimal minúscula.
pub fn sha256_hex(contenido: &[u8]) -> String {
    hex::encode(Sha256::digest(contenido))
}

/// Valida el contenido (tipo por *magic bytes* y tamaño), guarda los bytes y registra el
/// archivo con su auditoría.
///
/// Los bytes se guardan **antes** de la transacción: si luego falla el registro, se borran. En
/// el peor caso (caída entre ambos pasos) queda un archivo huérfano en el almacén, nunca una
/// fila que apunte a nada.
///
/// # Errors
///
/// - [`AppError::Dominio`] si el contenido está vacío, no es de un tipo admitido o es
///   demasiado grande.
/// - [`AppError::Infraestructura`] si falla el almacén o la base de datos.
pub async fn subir_archivo<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    subido: ArchivoSubido,
) -> AppResult<Archivo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos + RegistroAuditoria,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::ArchivosSubir)?;
    let tipo = TipoArchivo::validar(&subido.contenido)?;
    let id = ArchivoId::nuevo();
    let archivo = Archivo {
        id,
        clave: ClaveArchivo::new(ctx.tenant_id, id),
        almacen: almacen.almacen(),
        nombre_original: subido.nombre_original,
        tipo,
        tamano_bytes: subido.contenido.len() as u64,
        sha256: sha256_hex(&subido.contenido),
        clasificacion: subido.clasificacion,
        subido_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };

    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    if let Err(e) = registrar(unidad, ctx, &archivo).await {
        if let Err(limpieza) = almacen.eliminar(&archivo.clave).await {
            tracing::warn!(
                tenant_id = %ctx.tenant_id,
                archivo_id = %archivo.id,
                error = %limpieza,
                "no se pudo borrar el archivo huérfano"
            );
        }
        return Err(e);
    }
    Ok(archivo)
}

async fn registrar<U>(unidad: &U, ctx: &Contexto, archivo: &Archivo) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos + RegistroAuditoria,
{
    use crate::puertos::Transaccion as _;

    // El nombre original no se audita: puede contener datos personales.
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", archivo.tipo.as_str())?
        .con("tamano_bytes", archivo.tamano_bytes)?
        .con("clasificacion", archivo.clasificacion.as_str())?;
    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar(archivo).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_SUBIR, "archivo", archivo.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// URL temporal para descargar el archivo `id` de la copropiedad del contexto, válida por
/// `vigencia`.
///
/// # Errors
///
/// - [`AppError::NoEncontrado`] si el archivo no existe en la copropiedad.
/// - [`AppError::Infraestructura`] si el archivo está en otro almacén que el configurado (una
///   migración a medias: se termina antes de cambiar `ARCHIVOS_ALMACEN`) o si falla la BD.
pub async fn enlace_descarga<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ArchivoId,
    vigencia: Duration,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::ArchivosVer)?;
    // Solo lectura: la transacción se suelta sin confirmar.
    let archivo = unidad
        .iniciar(ctx)
        .await?
        .buscar(id)
        .await?
        .ok_or(AppError::NoEncontrado("archivo"))?;
    if matches!(
        archivo.clasificacion,
        ClasificacionArchivo::Personal | ClasificacionArchivo::Sensible
    ) {
        ctx.exigir(Permiso::ArchivosPersonalesVer)?;
    }
    if archivo.almacen != almacen.almacen() {
        return Err(AppError::infra(AlmacenDistinto {
            archivo: archivo.almacen,
            configurado: almacen.almacen(),
        }));
    }
    let vigencia = chrono::Duration::from_std(vigencia).map_err(AppError::infra)?;
    almacen.url_temporal(&DescargaArchivo {
        clave: archivo.clave,
        tipo: archivo.tipo,
        nombre: archivo.nombre_original,
        vence_en: reloj.ahora() + vigencia,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("el archivo está en el almacén `{archivo}` y el configurado es `{configurado}`")]
struct AlmacenDistinto {
    archivo: domain::Almacen,
    configurado: domain::Almacen,
}

/// Resultado de un lote de migración.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenMigracion {
    pub migrados: u32,
    /// Archivos que no se pudieron copiar o cuya copia no coincide con el SHA-256: siguen en
    /// el almacén de origen y se reintentan en el próximo lote.
    pub con_error: u32,
}

impl ResumenMigracion {
    /// No quedaba nada por migrar en esta copropiedad.
    pub const fn vacio(&self) -> bool {
        self.migrados == 0 && self.con_error == 0
    }
}

/// Mueve hasta `limite` archivos de la copropiedad del contexto de `origen` a `destino`.
///
/// Por cada archivo: lee de `origen`, verifica el SHA-256 registrado, copia a `destino`,
/// vuelve a leer la copia y la verifica, y solo entonces cambia `almacen` en la fila. **No
/// borra el origen**: se limpia a mano cuando toda la migración esté verificada. Mientras tanto
/// cada descarga sale del almacén que diga la fila.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos. Los fallos de un archivo concreto
/// no detienen el lote: se cuentan en [`ResumenMigracion::con_error`].
pub async fn migrar_lote<U, O, D>(
    unidad: &U,
    origen: &O,
    destino: &D,
    ctx: &Contexto,
    limite: u32,
) -> AppResult<ResumenMigracion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioArchivos + RegistroAuditoria,
    O: AlmacenArchivos,
    D: AlmacenArchivos,
{
    use crate::puertos::Transaccion as _;

    let mut tx = unidad.iniciar(ctx).await?;
    let mut resumen = ResumenMigracion::default();
    for archivo in tx.en_almacen(origen.almacen(), limite).await? {
        match copiar_verificado(&archivo, origen, destino).await {
            Ok(()) => {
                tx.cambiar_almacen(archivo.id, destino.almacen()).await?;
                let detalle = DetalleAuditoria::vacio()
                    .con("desde", origen.almacen().as_str())?
                    .con("hacia", destino.almacen().as_str())?;
                tx.registrar(
                    EventoAuditoria::ok(ACCION_MIGRAR, "archivo", archivo.id.as_uuid())
                        .con_detalle(detalle),
                )
                .await?;
                resumen.migrados += 1;
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %ctx.tenant_id,
                    archivo_id = %archivo.id,
                    error = %e,
                    "no se pudo migrar el archivo"
                );
                resumen.con_error += 1;
            }
        }
    }
    tx.confirmar().await?;
    Ok(resumen)
}

#[derive(Debug, thiserror::Error)]
#[error("el SHA-256 de la {0} no coincide con el registrado")]
struct HuellaDistinta(&'static str);

async fn copiar_verificado(
    archivo: &Archivo,
    origen: &impl AlmacenArchivos,
    destino: &impl AlmacenArchivos,
) -> AppResult<()> {
    let contenido = origen.obtener(&archivo.clave).await?;
    if sha256_hex(&contenido) != archivo.sha256 {
        return Err(AppError::infra(HuellaDistinta("lectura del origen")));
    }
    destino.guardar(&archivo.clave, &contenido).await?;
    if sha256_hex(&destino.obtener(&archivo.clave).await?) != archivo.sha256 {
        return Err(AppError::infra(HuellaDistinta("copia en el destino")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use domain::{Almacen, DominioError, TenantId};

    use super::*;
    use crate::puertos::{
        archivos::memoria::AlmacenEnMemoria, unidad_trabajo::memoria::UnidadEnMemoria,
    };

    struct RelojFijo;

    impl Reloj for RelojFijo {
        fn ahora(&self) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 11, 6, 15, 0, 0)
                .single()
                .expect("fecha válida")
        }
    }

    fn pdf() -> ArchivoSubido {
        ArchivoSubido {
            nombre_original: NombreArchivo::new("Reglamento PH.pdf").expect("válido"),
            clasificacion: ClasificacionArchivo::Publico,
            contenido: b"%PDF-1.7\nreglamento".to_vec(),
        }
    }

    #[tokio::test]
    async fn subir_guarda_bytes_fila_y_auditoria_sin_el_nombre() {
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);
        let ctx = Contexto::new(TenantId::nuevo());

        let archivo = subir_archivo(&unidad, &almacen, &RelojFijo, &ctx, pdf())
            .await
            .expect("se sube");

        assert_eq!(archivo.tipo, TipoArchivo::Pdf);
        assert_eq!(archivo.almacen, Almacen::Local);
        assert_eq!(archivo.tamano_bytes, 19);
        assert_eq!(archivo.sha256, sha256_hex(b"%PDF-1.7\nreglamento"));
        assert!(
            archivo
                .clave
                .as_str()
                .starts_with(&ctx.tenant_id.to_string())
        );
        assert_eq!(almacen.claves(), vec![archivo.clave.as_str().to_owned()]);
        assert_eq!(unidad.archivos(), vec![archivo.clone()]);

        let auditoria = unidad.confirmados();
        assert_eq!(auditoria.len(), 1);
        assert_eq!(auditoria[0].1.accion, ACCION_SUBIR);
        let detalle = auditoria[0].1.detalle.a_json().to_string();
        assert!(detalle.contains("application/pdf"), "{detalle}");
        assert!(!detalle.contains("Reglamento"), "{detalle}");
    }

    #[tokio::test]
    async fn contenido_no_admitido_no_toca_el_almacen() {
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);
        let mut html = pdf();
        html.contenido = b"<html>no soy un pdf</html>".to_vec();

        let err = subir_archivo(
            &unidad,
            &almacen,
            &RelojFijo,
            &Contexto::new(TenantId::nuevo()),
            html,
        )
        .await
        .expect_err("se rechaza");

        assert!(matches!(
            err,
            AppError::Dominio(DominioError::TipoArchivoNoAdmitido)
        ));
        assert!(almacen.claves().is_empty());
        assert!(unidad.archivos().is_empty());
    }

    #[tokio::test]
    async fn si_falla_el_registro_se_borran_los_bytes() {
        let unidad = UnidadEnMemoria::que_falla_al_confirmar();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);

        let res = subir_archivo(
            &unidad,
            &almacen,
            &RelojFijo,
            &Contexto::new(TenantId::nuevo()),
            pdf(),
        )
        .await;

        assert!(matches!(res, Err(AppError::Infraestructura(_))));
        assert!(almacen.claves().is_empty(), "no deja huérfanos");
    }

    #[tokio::test]
    async fn si_falla_el_almacen_no_se_registra_nada() {
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria {
            falla_al_guardar: true,
            ..AlmacenEnMemoria::new(Almacen::Local)
        };

        let res = subir_archivo(
            &unidad,
            &almacen,
            &RelojFijo,
            &Contexto::new(TenantId::nuevo()),
            pdf(),
        )
        .await;

        assert!(matches!(res, Err(AppError::Infraestructura(_))));
        assert!(unidad.archivos().is_empty());
        assert!(unidad.confirmados().is_empty());
    }

    #[tokio::test]
    async fn enlace_de_descarga_vence_segun_la_vigencia() {
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);
        let ctx = Contexto::new(TenantId::nuevo());
        let archivo = subir_archivo(&unidad, &almacen, &RelojFijo, &ctx, pdf())
            .await
            .expect("se sube");

        let url = enlace_descarga(
            &unidad,
            &almacen,
            &RelojFijo,
            &ctx,
            archivo.id,
            VIGENCIA_ENLACE,
        )
        .await
        .expect("enlace");

        let vence = RelojFijo.ahora().timestamp() + 300;
        assert_eq!(url, format!("memoria://{}?vence={vence}", archivo.clave));
    }

    #[tokio::test]
    async fn enlace_de_archivo_inexistente_es_no_encontrado() {
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);

        let err = enlace_descarga(
            &unidad,
            &almacen,
            &RelojFijo,
            &Contexto::new(TenantId::nuevo()),
            ArchivoId::nuevo(),
            VIGENCIA_ENLACE,
        )
        .await
        .expect_err("no existe");

        assert!(matches!(err, AppError::NoEncontrado("archivo")));
    }

    #[tokio::test]
    async fn migrar_copia_verifica_y_cambia_el_almacen_sin_borrar_el_origen() {
        let unidad = UnidadEnMemoria::default();
        let local = AlmacenEnMemoria::new(Almacen::Local);
        let s3 = AlmacenEnMemoria::new(Almacen::S3);
        let ctx = Contexto::new(TenantId::nuevo());
        for _ in 0..3 {
            subir_archivo(&unidad, &local, &RelojFijo, &ctx, pdf())
                .await
                .expect("se sube");
        }

        let primero = migrar_lote(&unidad, &local, &s3, &ctx, 2)
            .await
            .expect("migra");
        let segundo = migrar_lote(&unidad, &local, &s3, &ctx, 2)
            .await
            .expect("migra");
        let tercero = migrar_lote(&unidad, &local, &s3, &ctx, 2)
            .await
            .expect("migra");

        assert_eq!((primero.migrados, segundo.migrados), (2, 1));
        assert!(tercero.vacio());
        assert!(unidad.archivos().iter().all(|a| a.almacen == Almacen::S3));
        assert_eq!(s3.claves(), local.claves(), "el origen se conserva");
        // Ahora se descarga desde S3 y ya no desde el disco local.
        let id = unidad.archivos()[0].id;
        assert!(
            enlace_descarga(&unidad, &s3, &RelojFijo, &ctx, id, VIGENCIA_ENLACE)
                .await
                .is_ok()
        );
        assert!(
            enlace_descarga(&unidad, &local, &RelojFijo, &ctx, id, VIGENCIA_ENLACE)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn sin_permiso_no_se_sube_ni_se_ve_un_archivo_personal() {
        use domain::{ConjuntoPermisos, CuentaId};
        let unidad = UnidadEnMemoria::default();
        let almacen = AlmacenEnMemoria::new(Almacen::Local);
        let tenant = TenantId::nuevo();
        let solo_ver = Contexto::de_cuenta(
            tenant,
            CuentaId::nuevo(),
            [Permiso::ArchivosVer]
                .into_iter()
                .collect::<ConjuntoPermisos>(),
        );
        let mut personal = pdf();
        personal.clasificacion = ClasificacionArchivo::Personal;
        let archivo = subir_archivo(
            &unidad,
            &almacen,
            &RelojFijo,
            &Contexto::new(tenant),
            personal,
        )
        .await
        .expect("el sistema sube");

        let subir = subir_archivo(&unidad, &almacen, &RelojFijo, &solo_ver, pdf()).await;
        let ver = enlace_descarga(
            &unidad,
            &almacen,
            &RelojFijo,
            &solo_ver,
            archivo.id,
            VIGENCIA_ENLACE,
        )
        .await;

        assert!(matches!(
            subir,
            Err(AppError::SinPermiso(Permiso::ArchivosSubir))
        ));
        assert!(matches!(
            ver,
            Err(AppError::SinPermiso(Permiso::ArchivosPersonalesVer))
        ));
    }

    #[tokio::test]
    async fn migrar_no_mueve_un_archivo_corrupto() {
        let unidad = UnidadEnMemoria::default();
        let local = AlmacenEnMemoria::new(Almacen::Local);
        let s3 = AlmacenEnMemoria::new(Almacen::S3);
        let ctx = Contexto::new(TenantId::nuevo());
        let archivo = subir_archivo(&unidad, &local, &RelojFijo, &ctx, pdf())
            .await
            .expect("se sube");
        local
            .contenido
            .lock()
            .expect("mutex sano")
            .insert(archivo.clave.as_str().to_owned(), b"%PDF-alterado".to_vec());

        let resumen = migrar_lote(&unidad, &local, &s3, &ctx, 10)
            .await
            .expect("lote");

        assert_eq!(
            resumen,
            ResumenMigracion {
                migrados: 0,
                con_error: 1
            }
        );
        assert_eq!(unidad.archivos()[0].almacen, Almacen::Local);
    }
}
