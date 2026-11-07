//! Servicio HTTP de DAD_COPROPIEDAD: API REST (y, más adelante, WebSocket de resultados).
//!
//! Es la **raíz de composición** de la arquitectura hexagonal: conecta los adaptadores
//! (`infra-*`) con los casos de uso de `application` y traduce HTTP ↔ casos de uso. No contiene
//! reglas de negocio.
//!
//! Se compila como biblioteca (este crate) más el binario `dad_copropiedad_api`, para que las
//! pruebas de `tests/` usen el router y el contrato OpenAPI igual que el binario.
//!
//! - [`routes`]: router, middlewares y convención de rutas.
//! - [`error`]: error uniforme y catálogo de códigos.
//! - [`extractores`]: `Json`, `Query` y `Path` que responden con el error uniforme.
//! - [`paginacion`]: DTOs de la paginación por cursor.
//! - [`openapi`]: metadatos del contrato OpenAPI.
//! - [`sesion`]: extractores de autenticación y autorización.

pub mod error;
pub mod extractores;
pub mod openapi;
pub mod paginacion;
pub mod routes;
pub mod sesion;
pub mod state;
