//! Parqueaderos: inventario, naturaleza y asignaciones.

use serde::{Deserialize, Serialize};

/// Vehículo para el que está destinado el parqueadero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoParqueadero {
    Carro,
    Moto,
    Bicicleta,
    /// Reservado para personas con movilidad reducida.
    Discapacidad,
    /// Con punto de carga.
    Electrico,
}

enum_texto!(TipoParqueadero {
    Carro => "carro",
    Moto => "moto",
    Bicicleta => "bicicleta",
    Discapacidad => "discapacidad",
    Electrico => "electrico",
});

/// Naturaleza jurídica y de uso del parqueadero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalezaParqueadero {
    /// Es un bien privado (plan 03) o está vinculado a uno.
    Privado,
    /// Bien común asignado en uso exclusivo a un bien privado (art. 22).
    UsoExclusivo,
    /// Se asigna por sorteo o rotación por periodos.
    Comunal,
    /// Reservas cortas de visitantes.
    Visitantes,
}

enum_texto!(NaturalezaParqueadero {
    Privado => "privado",
    UsoExclusivo => "uso_exclusivo",
    Comunal => "comunal",
    Visitantes => "visitantes",
});

/// De dónde sale la asignación de un parqueadero a un bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenAsignacionParqueadero {
    Propiedad,
    Sorteo,
    Arriendo,
    Manual,
}

enum_texto!(OrigenAsignacionParqueadero {
    Propiedad => "propiedad",
    Sorteo => "sorteo",
    Arriendo => "arriendo",
    Manual => "manual",
});

/// Estado de un sorteo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoSorteo {
    /// Recibe inscripciones; el compromiso ya está publicado.
    Abierto,
    /// Semilla revelada y resultado aplicado.
    Ejecutado,
}

enum_texto!(EstadoSorteo {
    Abierto => "abierto",
    Ejecutado => "ejecutado",
});
