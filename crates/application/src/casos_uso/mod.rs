//! Casos de uso. Cada uno es genérico sobre los puertos que necesita.

pub mod acceso;
pub mod archivos;
pub mod asambleas;
pub mod autenticacion;
pub mod avisos;
pub mod bienes_privados;
pub mod censo;
pub mod chat;
pub mod coeficientes;
pub mod conversaciones;
pub mod copropiedades;
pub mod despachar_salida;
pub mod importaciones;
pub mod mantenimiento;
pub mod modulos;
pub mod organizaciones;
pub mod parqueaderos;
pub mod plataforma;
pub mod proteccion_datos;
pub mod reservas;
pub mod roles;
pub mod tickets;
pub mod titulares;

pub use archivos::{ArchivoSubido, ResumenMigracion, enlace_descarga, migrar_lote, subir_archivo};
pub use despachar_salida::{Despachadores, ResumenDespacho, despachar_lote};
