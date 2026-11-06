//! Generación de actas de asamblea en PDF con Typst.
//!
//! Implementará [`application::puertos::GeneradorActas`]. Lineamientos:
//! - Plantilla Typst versionada en el repositorio; el adaptador solo inyecta datos ya
//!   calculados ([`application::puertos::actas::DatosActa`]): no decide quórum ni resultados.
//! - Para votaciones nominales el acta lista bien privado, coeficiente, canal, hora y apoderado
//!   (tomados de `participaciones`); en secretas, solo totales por opción.
//! - Se ejecuta en el worker (inicialmente el binario `gateway`), nunca en la ruta de votación.
//!
//! Se deja vacío a propósito hasta la porción vertical de actas, para no arrastrar la
//! dependencia de Typst antes de necesitarla.
