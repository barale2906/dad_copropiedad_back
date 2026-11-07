//! Libros del censo (Ley 675, art. 51): residentes, vehículos y mascotas de cada bien privado.
//!
//! - Quién vive en un bien privado y en qué calidad ([`CalidadResidencia`]); solo algunas
//!   calidades reciben el rol derivado `residente`.
//! - Placas normalizadas ([`Placa`]): una placa vigente por copropiedad.
//! - Mascotas con sus vencimientos (vacunas, póliza de las razas potencialmente peligrosas,
//!   Ley 1801 de 2016) y las alertas que de ellos salen ([`alertas_mascota`]).
//! - Propuestas de cambio por autoservicio ([`EstadoCambioCenso`]).

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Calidad en que una persona reside en un bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalidadResidencia {
    /// Titular vigente que vive en su bien.
    PropietarioResidente,
    /// Arrendatario u otro título (Ley 675: tenedor). Lo autoriza un propietario.
    Tenedor,
    Familiar,
    HuespedTemporal,
    EmpleadoDomestico,
}

enum_texto!(CalidadResidencia {
    PropietarioResidente => "propietario_residente",
    Tenedor => "tenedor",
    Familiar => "familiar",
    HuespedTemporal => "huesped_temporal",
    EmpleadoDomestico => "empleado_domestico",
});

impl CalidadResidencia {
    /// ¿Recibe el rol `residente` si tiene cuenta? Huéspedes temporales y empleados
    /// domésticos no (no reservan bienes comunes ni autorizan visitantes).
    pub const fn recibe_rol_residente(self) -> bool {
        matches!(
            self,
            Self::PropietarioResidente | Self::Tenedor | Self::Familiar
        )
    }
}

/// Tipo de documento de identidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TipoDocumento {
    /// Cédula de ciudadanía.
    #[serde(rename = "CC")]
    Cc,
    /// Cédula de extranjería.
    #[serde(rename = "CE")]
    Ce,
    /// Tarjeta de identidad (menores de 7 a 17 años).
    #[serde(rename = "TI")]
    Ti,
    /// Registro civil (menores de 7 años).
    #[serde(rename = "RC")]
    Rc,
    /// Pasaporte.
    #[serde(rename = "PA")]
    Pa,
    /// Permiso por protección temporal (migrantes venezolanos).
    #[serde(rename = "PPT")]
    Ppt,
    #[serde(rename = "NIT")]
    Nit,
}

enum_texto!(TipoDocumento {
    Cc => "CC",
    Ce => "CE",
    Ti => "TI",
    Rc => "RC",
    Pa => "PA",
    Ppt => "PPT",
    Nit => "NIT",
});

/// ¿Es menor de edad (18 años) en `hoy`? Sus datos tienen tratamiento especial (Ley 1581,
/// art. 7).
pub fn es_menor(fecha_nacimiento: NaiveDate, hoy: NaiveDate) -> bool {
    let mut edad = hoy.year() - fecha_nacimiento.year();
    if (hoy.month(), hoy.day()) < (fecha_nacimiento.month(), fecha_nacimiento.day()) {
        edad -= 1;
    }
    edad < 18
}

/// Clase de vehículo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoVehiculo {
    Carro,
    Moto,
    Bicicleta,
    Otro,
}

enum_texto!(TipoVehiculo {
    Carro => "carro",
    Moto => "moto",
    Bicicleta => "bicicleta",
    Otro => "otro",
});

impl TipoVehiculo {
    /// Carros y motos siempre tienen placa.
    pub const fn exige_placa(self) -> bool {
        matches!(self, Self::Carro | Self::Moto)
    }
}

/// Placa normalizada: mayúsculas, sin espacios, guiones ni puntos, de 4 a 8 letras o dígitos
/// (`ABC123`, `ABC12D`). Identifica al dueño del vehículo: `Debug` la oculta.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Placa(String);

impl Placa {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no quedan de 4 a 8 letras o dígitos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let limpia: String = valor
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '.'))
            .collect::<String>()
            .to_uppercase();
        if (4..=8).contains(&limpia.len()) && limpia.chars().all(|c| c.is_ascii_alphanumeric()) {
            Ok(Self(limpia))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "placa",
                regla: "de 4 a 8 letras o dígitos, p. ej. ABC123",
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Placa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Placa(<oculta>)")
    }
}

impl TryFrom<String> for Placa {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<Placa> for String {
    fn from(p: Placa) -> Self {
        p.0
    }
}

/// Valida la placa según el tipo de vehículo.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si un carro o una moto no tienen placa.
pub fn exigir_placa(tipo: TipoVehiculo, placa: Option<&Placa>) -> Result<(), DominioError> {
    if tipo.exige_placa() && placa.is_none() {
        return Err(DominioError::DatoInvalido {
            campo: "placa",
            regla: "los carros y las motos deben tener placa",
        });
    }
    Ok(())
}

/// Especie de la mascota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EspecieMascota {
    Perro,
    Gato,
    Otro,
}

enum_texto!(EspecieMascota {
    Perro => "perro",
    Gato => "gato",
    Otro => "otro",
});

/// Estado de la mascota en el censo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoMascota {
    Activa,
    Fallecida,
    /// Ya no vive en la copropiedad.
    Trasladada,
}

enum_texto!(EstadoMascota {
    Activa => "activa",
    Fallecida => "fallecida",
    Trasladada => "trasladada",
});

impl EstadoMascota {
    /// Solo una mascota activa cambia de estado (a fallecida o trasladada).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] en otro caso.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        match (self, hacia) {
            (Self::Activa, Self::Fallecida | Self::Trasladada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "mascota",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Lo que se revisa de una mascota para sus alertas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VencimientosMascota {
    pub estado: EstadoMascota,
    pub potencialmente_peligrosa: bool,
    pub tiene_poliza: bool,
    pub poliza_vence: Option<NaiveDate>,
    pub vacunas_vencen: Option<NaiveDate>,
}

/// Alerta del censo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertaMascota {
    VacunasVencidas,
    /// Vencen en los próximos [`DIAS_AVISO`] días.
    VacunasPorVencer,
    SinRegistroDeVacunas,
    /// Raza potencialmente peligrosa sin póliza de responsabilidad civil (Ley 1801, art. 127).
    SinPoliza,
    PolizaVencida,
    PolizaPorVencer,
}

enum_texto!(AlertaMascota {
    VacunasVencidas => "vacunas_vencidas",
    VacunasPorVencer => "vacunas_por_vencer",
    SinRegistroDeVacunas => "sin_registro_de_vacunas",
    SinPoliza => "sin_poliza",
    PolizaVencida => "poliza_vencida",
    PolizaPorVencer => "poliza_por_vencer",
});

/// Anticipación de los avisos de vencimiento.
pub const DIAS_AVISO: i64 = 30;

fn vencimiento(
    fecha: NaiveDate,
    hoy: NaiveDate,
    vencida: AlertaMascota,
    por_vencer: AlertaMascota,
) -> Option<AlertaMascota> {
    let dias = (fecha - hoy).num_days();
    if dias < 0 {
        Some(vencida)
    } else if dias <= DIAS_AVISO {
        Some(por_vencer)
    } else {
        None
    }
}

/// Alertas de una mascota en `hoy`. Las inactivas no generan alertas.
pub fn alertas_mascota(m: &VencimientosMascota, hoy: NaiveDate) -> Vec<AlertaMascota> {
    if m.estado != EstadoMascota::Activa {
        return Vec::new();
    }
    let mut v = Vec::new();
    match m.vacunas_vencen {
        None => v.push(AlertaMascota::SinRegistroDeVacunas),
        Some(f) => v.extend(vencimiento(
            f,
            hoy,
            AlertaMascota::VacunasVencidas,
            AlertaMascota::VacunasPorVencer,
        )),
    }
    if m.potencialmente_peligrosa {
        match (m.tiene_poliza, m.poliza_vence) {
            (true, Some(f)) => v.extend(vencimiento(
                f,
                hoy,
                AlertaMascota::PolizaVencida,
                AlertaMascota::PolizaPorVencer,
            )),
            _ => v.push(AlertaMascota::SinPoliza),
        }
    }
    v
}

/// Qué se propone cambiar en el censo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCambioCenso {
    Residencia,
    Vehiculo,
    Mascota,
}

enum_texto!(TipoCambioCenso {
    Residencia => "residencia",
    Vehiculo => "vehiculo",
    Mascota => "mascota",
});

/// Estado de una propuesta de cambio del censo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCambioCenso {
    Pendiente,
    Aprobado,
    Rechazado,
}

enum_texto!(EstadoCambioCenso {
    Pendiente => "pendiente",
    Aprobado => "aprobado",
    Rechazado => "rechazado",
});

impl EstadoCambioCenso {
    /// Una propuesta se resuelve una sola vez.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si ya estaba resuelta.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        match (self, hacia) {
            (Self::Pendiente, Self::Aprobado | Self::Rechazado) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "cambio del censo",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(a: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, dd).expect("fecha válida")
    }

    #[test]
    fn mayoria_de_edad_se_cumple_el_dia_del_cumpleanos() {
        assert!(es_menor(d(2008, 11, 8), d(2026, 11, 7)));
        assert!(!es_menor(d(2008, 11, 7), d(2026, 11, 7)));
        assert!(!es_menor(d(1980, 1, 1), d(2026, 11, 7)));
    }

    #[test]
    fn placas_se_normalizan() {
        assert_eq!(Placa::new("abc-123").expect("válida").as_str(), "ABC123");
        assert_eq!(Placa::new(" abc 12d ").expect("válida").as_str(), "ABC12D");
        for invalida in ["", "AB1", "ABC#123", "ABCDEFGHI"] {
            assert!(Placa::new(invalida).is_err(), "{invalida:?}");
        }
        assert!(!format!("{:?}", Placa::new("ABC123").expect("válida")).contains("ABC"));
    }

    #[test]
    fn carros_y_motos_exigen_placa() {
        assert!(exigir_placa(TipoVehiculo::Carro, None).is_err());
        assert!(exigir_placa(TipoVehiculo::Bicicleta, None).is_ok());
        let p = Placa::new("ABC123").expect("válida");
        assert!(exigir_placa(TipoVehiculo::Moto, Some(&p)).is_ok());
    }

    #[test]
    fn solo_residentes_permanentes_reciben_el_rol() {
        assert!(CalidadResidencia::Tenedor.recibe_rol_residente());
        assert!(CalidadResidencia::Familiar.recibe_rol_residente());
        assert!(!CalidadResidencia::HuespedTemporal.recibe_rol_residente());
        assert!(!CalidadResidencia::EmpleadoDomestico.recibe_rol_residente());
    }

    fn mascota() -> VencimientosMascota {
        VencimientosMascota {
            estado: EstadoMascota::Activa,
            potencialmente_peligrosa: false,
            tiene_poliza: false,
            poliza_vence: None,
            vacunas_vencen: Some(d(2027, 6, 1)),
        }
    }

    #[test]
    fn alertas_de_vacunas() {
        let hoy = d(2026, 11, 7);
        assert!(alertas_mascota(&mascota(), hoy).is_empty());
        let vencida = VencimientosMascota {
            vacunas_vencen: Some(d(2026, 11, 6)),
            ..mascota()
        };
        assert_eq!(
            alertas_mascota(&vencida, hoy),
            vec![AlertaMascota::VacunasVencidas]
        );
        let pronto = VencimientosMascota {
            vacunas_vencen: Some(d(2026, 12, 7)),
            ..mascota()
        };
        assert_eq!(
            alertas_mascota(&pronto, hoy),
            vec![AlertaMascota::VacunasPorVencer]
        );
        let sin = VencimientosMascota {
            vacunas_vencen: None,
            ..mascota()
        };
        assert_eq!(
            alertas_mascota(&sin, hoy),
            vec![AlertaMascota::SinRegistroDeVacunas]
        );
    }

    #[test]
    fn raza_peligrosa_exige_poliza_vigente() {
        let hoy = d(2026, 11, 7);
        let sin_poliza = VencimientosMascota {
            potencialmente_peligrosa: true,
            ..mascota()
        };
        assert_eq!(
            alertas_mascota(&sin_poliza, hoy),
            vec![AlertaMascota::SinPoliza]
        );
        let vencida = VencimientosMascota {
            tiene_poliza: true,
            poliza_vence: Some(d(2026, 1, 1)),
            ..sin_poliza
        };
        assert_eq!(
            alertas_mascota(&vencida, hoy),
            vec![AlertaMascota::PolizaVencida]
        );
        let fallecida = VencimientosMascota {
            estado: EstadoMascota::Fallecida,
            ..vencida
        };
        assert!(alertas_mascota(&fallecida, hoy).is_empty());
    }

    #[test]
    fn maquinas_de_estado() {
        assert!(
            EstadoMascota::Activa
                .transicionar(EstadoMascota::Trasladada)
                .is_ok()
        );
        assert!(
            EstadoMascota::Fallecida
                .transicionar(EstadoMascota::Activa)
                .is_err()
        );
        assert!(
            EstadoCambioCenso::Pendiente
                .transicionar(EstadoCambioCenso::Aprobado)
                .is_ok()
        );
        assert!(
            EstadoCambioCenso::Rechazado
                .transicionar(EstadoCambioCenso::Aprobado)
                .is_err()
        );
    }

    #[test]
    fn textos_ida_y_vuelta() {
        for t in TipoDocumento::TODOS {
            assert_eq!(TipoDocumento::try_from(t.as_str()), Ok(*t));
            assert_eq!(
                serde_json::to_value(t).expect("serializa"),
                serde_json::json!(t.as_str())
            );
        }
    }
}
