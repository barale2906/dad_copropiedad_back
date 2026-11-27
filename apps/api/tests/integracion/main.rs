//! Pruebas de integración del crate en un solo ejecutable.
//!
//! Cada archivo de `tests/` sería un ejecutable aparte que enlaza todo el stack; juntos en
//! un módulo por archivo se enlaza una vez (menos disco y menos tiempo de enlazado).
//! Una prueba nueva va como módulo aquí, no como archivo suelto en `tests/`.

mod acceso_api;
mod asambleas_api;
mod bloque2_api;
mod busqueda_proveedores_api;
mod cartera_api;
mod censo_api;
mod cierre_api;
mod cobros_api;
mod comun;
mod contabilidad_api;
mod contrato_openapi;
mod copropiedades_api;
mod descarga_archivos;
mod directorio_api;
mod facturacion_api;
mod gestion_api;
mod identidad_api;
mod nomina_api;
mod peticiones_invalidas;
mod plataforma_entidades_api;
mod publicidad_api;
mod reservas_api;
mod roles_api;
mod seguridad_cuentas_api;
mod tesoreria_api;
mod tributario_api;
mod usuarios_api;
