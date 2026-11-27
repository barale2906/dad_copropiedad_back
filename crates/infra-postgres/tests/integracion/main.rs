//! Pruebas de integración del crate en un solo ejecutable.
//!
//! Cada archivo de `tests/` sería un ejecutable aparte que enlaza todo el stack; juntos en
//! un módulo por archivo se enlaza una vez (menos disco y menos tiempo de enlazado).
//! Una prueba nueva va como módulo aquí, no como archivo suelto en `tests/`.

mod acceso;
mod archivos;
mod asambleas;
mod auditoria;
mod busqueda_proveedores;
mod cartera;
mod censo;
mod cobros;
mod comun;
mod comunicados;
mod contabilidad;
mod copropiedades;
mod directorio;
mod entidad_generica;
mod esquema;
mod facturacion;
mod gestion;
mod identidad;
mod modulos_y_busqueda;
mod nomina;
mod outbox;
mod proteccion_datos;
mod publicidad;
mod reservas;
mod tesoreria;
mod tributario;
