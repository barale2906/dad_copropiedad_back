//! Contrato OpenAPI de la API REST.
//!
//! Las rutas se registran con `utoipa_axum::routes!` en [`crate::routes`], así que el documento
//! se genera desde los mismos handlers que atienden las peticiones: no puede haber una ruta
//! servida y no documentada. Aquí solo se declaran los metadatos, las etiquetas por módulo y los
//! esquemas comunes.
//!
//! El documento generado se versiona en `apps/api/openapi.json`; la prueba
//! `tests/contrato_openapi.rs` falla si cambia sin regenerarlo (`make openapi`).

use utoipa::OpenApi;

use crate::error::{CodigoError, DetalleError, ErrorRespuesta};

/// Etiqueta de las sondas de salud.
pub const ETIQUETA_SALUD: &str = "salud";
/// Etiqueta de archivos y adjuntos.
pub const ETIQUETA_ARCHIVOS: &str = "archivos";

/// Metadatos del documento. Las rutas las agrega [`crate::routes::openapi`] a partir del router.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "DAD_COPROPIEDAD API",
        description = "API REST para la gestión de copropiedades sometidas al régimen de \
                       propiedad horizontal (Ley 675 de 2001).\n\n\
                       - Rutas de una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/…`\n\
                       - Rutas de la cuenta autenticada: `/api/v1/yo/…`\n\
                       - Rutas globales de la plataforma: `/api/v1/plataforma/…`\n\
                       - Descarga con enlace firmado: `/api/v1/archivos/descarga`\n\n\
                       Errores: `{ \"error\": { \"codigo\": \"…\", \"mensaje\": \"…\" } }` \
                       (ver el esquema `CodigoError`). Listados: paginación por cursor con \
                       `?limite=&despues=`."
    ),
    tags(
        (name = ETIQUETA_SALUD, description = "Sondas de vida y disponibilidad del servicio"),
        (name = ETIQUETA_ARCHIVOS, description = "Archivos y adjuntos: descarga con enlaces \
                                                  firmados de vida corta"),
    ),
    components(schemas(ErrorRespuesta, DetalleError, CodigoError))
)]
pub struct ApiDoc;
