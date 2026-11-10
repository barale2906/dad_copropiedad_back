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
/// Etiqueta de autenticación.
pub const ETIQUETA_AUTH: &str = "autenticacion";
/// Etiqueta de la cuenta autenticada.
pub const ETIQUETA_CUENTA: &str = "cuenta";
/// Etiqueta de roles y permisos.
pub const ETIQUETA_ROLES: &str = "roles";
/// Etiqueta de organizaciones y contratos.
pub const ETIQUETA_ORGANIZACIONES: &str = "organizaciones";
/// Etiqueta de la ficha de la copropiedad (datos, cargos, puntos de atención).
pub const ETIQUETA_COPROPIEDAD: &str = "copropiedad";
/// Etiqueta de agrupaciones, bienes privados, usos exclusivos y titulares.
pub const ETIQUETA_BIENES_PRIVADOS: &str = "bienes_privados";
/// Etiqueta del censo: residentes, vehículos, mascotas, personas y libros.
pub const ETIQUETA_CENSO: &str = "censo";
/// Etiqueta de bienes comunes, reservas, citas, parqueaderos y sorteos.
pub const ETIQUETA_RESERVAS: &str = "reservas";
/// Etiqueta de los tickets (PQRS y solicitudes).
pub const ETIQUETA_TICKETS: &str = "tickets";
/// Etiqueta del chat de WhatsApp: preguntas frecuentes y bandeja de conversaciones.
pub const ETIQUETA_CONVERSACIONES: &str = "conversaciones";
/// Etiqueta del inventario y el mantenimiento.
pub const ETIQUETA_MANTENIMIENTO: &str = "mantenimiento";
/// Etiqueta de los cuadros de coeficientes.
pub const ETIQUETA_COEFICIENTES: &str = "coeficientes";
/// Etiqueta de las importaciones masivas.
pub const ETIQUETA_IMPORTACIONES: &str = "importaciones";
/// Etiqueta de la operación de la plataforma.
/// Etiqueta de asambleas y votaciones.
pub const ETIQUETA_ASAMBLEAS: &str = "asambleas";
/// Etiqueta de la protección de datos personales (Ley 1581).
pub const ETIQUETA_PROTECCION_DATOS: &str = "proteccion_datos";
/// Etiqueta de la consulta de auditoría.
pub const ETIQUETA_AUDITORIA: &str = "auditoria";
/// Etiqueta del control de acceso, visitas y correspondencia.
pub const ETIQUETA_ACCESO: &str = "acceso";
/// Etiqueta de los comunicados de la administración.
pub const ETIQUETA_COMUNICADOS: &str = "comunicados";
/// Etiqueta de las operaciones de plataforma.
pub const ETIQUETA_PLATAFORMA: &str = "plataforma";

/// Registra el esquema de seguridad `bearer` (token de acceso opaco).
struct EsquemaBearer;

impl utoipa::Modify for EsquemaBearer {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
        let componentes = openapi.components.get_or_insert_with(Default::default);
        componentes.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Token de acceso de `POST /api/v1/auth/sesion` (opaco, 30 minutos)",
                    ))
                    .build(),
            ),
        );
        componentes.add_security_scheme(
            "puente",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Token de máquina del puente de acceso (se entrega al registrarlo en \
                         `POST /copropiedades/{copropiedad_id}/puentes`)",
                    ))
                    .build(),
            ),
        );
    }
}

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
                       - Descarga con enlace firmado: `/api/v1/archivos/descarga`\n\
                       - Puentes de acceso (token de máquina): `/api/v1/puentes/…`\n\n\
                       Errores: `{ \"error\": { \"codigo\": \"…\", \"mensaje\": \"…\" } }` \
                       (ver el esquema `CodigoError`). Listados: paginación por cursor con \
                       `?limite=&despues=`.\n\n\
                       Reintentos seguros: los `POST` con sesión aceptan la cabecera \
                       `Idempotency-Key` (1 a 200 caracteres). Durante 24 h, repetir la misma \
                       petición con la misma clave devuelve la respuesta original con \
                       `Idempotency-Replayed: true`; la misma clave con otra petición responde \
                       422 `idempotencia_reusada`."
    ),
    tags(
        (name = ETIQUETA_SALUD, description = "Sondas de vida y disponibilidad del servicio"),
        (name = ETIQUETA_ARCHIVOS, description = "Archivos y adjuntos: descarga con enlaces \
                                                  firmados de vida corta"),
        (name = ETIQUETA_AUTH, description = "Registro, inicio y cierre de sesión"),
        (name = ETIQUETA_CUENTA, description = "La cuenta autenticada"),
        (name = ETIQUETA_ROLES, description = "Roles, permisos y asignaciones"),
        (name = ETIQUETA_ORGANIZACIONES, description = "Organizaciones compartidas y contratos de servicio"),
        (name = ETIQUETA_COPROPIEDAD, description = "Ficha de la copropiedad: datos legales, cargos \
                                                     vigentes, advertencias y puntos de atención"),
        (name = ETIQUETA_BIENES_PRIVADOS, description = "Agrupaciones, bienes privados, anexos, usos \
                                                         exclusivos y titulares"),
        (name = ETIQUETA_COEFICIENTES, description = "Cuadros de coeficientes versionados (suma \
                                                      exacta = 1)"),
        (name = ETIQUETA_CENSO, description = "Censo: residentes, vehículos, mascotas, personas, \
                                               autoservicio con aprobación y libros (art. 51)"),
        (name = ETIQUETA_RESERVAS, description = "Bienes comunes, reservas, citas con la \
                                                  administración, parqueaderos y sorteos \
                                                  (módulo `bienes_comunes`)"),
        (name = ETIQUETA_TICKETS, description = "Tickets: peticiones, quejas, reclamos y solicitudes \
                                                con plazos en días u horas hábiles, notas internas, \
                                                reportes de convivencia confidenciales y encuesta \
                                                (módulo `gestion_administrativa`)"),
        (name = ETIQUETA_CONVERSACIONES, description = "Chat de WhatsApp: preguntas frecuentes y \
                                                        bandeja de conversaciones escaladas a la \
                                                        administración (módulo `gestion_administrativa`)"),
        (name = ETIQUETA_COMUNICADOS, description = "Comunicados de la administración: \
            audiencia (toda la PH, propietarios, residentes, torres, bienes o roles), finalidad \
            (Ley 1581), envío inmediato o programado por push, correo y WhatsApp (solo los \
            importantes y a quien no tiene la app), cartelera con lectura y «Enterado» (módulo \
            `gestion_administrativa`)"),
        (name = ETIQUETA_MANTENIMIENTO, description = "Inventario de activos con hoja de vida, planes \
                                                       preventivos, órdenes de trabajo y \
                                                       obligaciones periódicas (módulo \
                                                       `gestion_administrativa`)"),
        (name = ETIQUETA_IMPORTACIONES, description = "Importación masiva desde CSV/XLSX con vista \
                                                       previa"),
        (name = ETIQUETA_ASAMBLEAS, description = "Asambleas y votaciones (Ley 675, arts. 37–49): \
            convocatoria con padrón y constancias, mesa de registro asistido, poderes, morosos, \
            quórum en vivo, preguntas (sí/no, selección, elección por cociente), voto del \
            asistente por todos sus bienes, acta en PDF, reportes y papeleta virtual"),
        (name = ETIQUETA_ACCESO, description = "Control de acceso (plan 05): visitas con aviso \
            al primer respondiente y escalamiento (2 min a todos los residentes, 5 min llamada), \
            pre-autorizaciones con QR firmado, correspondencia con avisos cada 24 h y entrega con \
            firma o foto, minuta, bitácora, credenciales, autorizaciones, biometría con \
            consentimiento y la API de los puentes (`/api/v1/puentes`)"),
        (name = ETIQUETA_PROTECCION_DATOS, description = "Protección de datos personales (Ley 1581 \
            de 2012): política de tratamiento versionada, autorizaciones por finalidad, \
            consultas y reclamos del titular con plazos en días hábiles y prórroga, «mis datos»"),
        (name = ETIQUETA_AUDITORIA, description = "Consulta de la auditoría de la copropiedad \
            (solo lectura; nunca trae datos personales)"),
        (name = ETIQUETA_PLATAFORMA, description = "Operación de la plataforma (exige segundo factor)"),
    ),
    modifiers(&EsquemaBearer),
    components(schemas(ErrorRespuesta, DetalleError, CodigoError))
)]
pub struct ApiDoc;
