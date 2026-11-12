//! Prueba de carga de DAD_COPROPIEDAD: cuántas copropiedades aguanta un servidor operando a la
//! vez.
//!
//! No forma parte del producto ni de la imagen de producción: es una herramienta de operación
//! que se ejecuta contra un **entorno de prueba** con la misma configuración que producción
//! (`docs/despliegue.md` §9). Solo habla con la API por HTTP, como lo haría el frontend.
//!
//! - [`siembra`]: crea copropiedades, bienes, propietarios y asambleas instaladas.
//! - [`escenario`]: navegación simultánea y ráfaga de votación en todas las copropiedades.
//! - [`estadisticas`]: percentiles y peticiones por segundo por operación.
//! - [`informe`]: tabla legible del resultado.

pub mod cliente;
pub mod escenario;
pub mod estadisticas;
pub mod informe;
pub mod siembra;
