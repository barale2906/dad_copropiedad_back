//! Facturación (plan 18, ADR 0022): reglas propias del módulo que se vende sin la contabilidad.
//! Catálogo de productos y servicios, marca de pago manual de una factura y habilitación de la
//! entidad ante la DIAN con el motor propio (ambiente, estados, set de pruebas). Lo tributario (IVA,
//! documentos electrónicos) está en [`crate::tributario`].

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{DominioError, tributario::ClasificacionTributaria};

/// Medio de un pago que se marca a mano (sin tesorería).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MedioPagoManual {
    Efectivo,
    Transferencia,
    Consignacion,
    Tarjeta,
    Cheque,
    Pse,
    Otro,
}

enum_texto!(MedioPagoManual {
    Efectivo => "efectivo",
    Transferencia => "transferencia",
    Consignacion => "consignacion",
    Tarjeta => "tarjeta",
    Cheque => "cheque",
    Pse => "pse",
    Otro => "otro",
});

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

/// Código de un producto o servicio: 1 a 30 caracteres `A-Z a-z 0-9 . _ -`.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no cumple.
pub fn validar_codigo_item(codigo: &str) -> Result<String, DominioError> {
    let c = codigo.trim();
    let ok = (1..=30).contains(&c.len())
        && c.chars()
            .all(|x| x.is_ascii_alphanumeric() || matches!(x, '.' | '_' | '-'));
    if ok {
        Ok(c.to_owned())
    } else {
        Err(invalido(
            "codigo",
            "de 1 a 30 caracteres: letras, dígitos, punto, guion o guion bajo",
        ))
    }
}

/// Unidad de medida del catálogo de la DIAN (UN/ECE rec. 20): 1 a 3 mayúsculas o dígitos (`94`
/// = unidad, `HUR` = hora, `KGM` = kilogramo).
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no cumple.
pub fn validar_unidad(unidad: &str) -> Result<String, DominioError> {
    let u = unidad.trim();
    if (1..=3).contains(&u.len())
        && u.chars()
            .all(|x| x.is_ascii_uppercase() || x.is_ascii_digit())
    {
        Ok(u.to_owned())
    } else {
        Err(invalido(
            "unidad",
            "código DIAN de 1 a 3 mayúsculas o dígitos",
        ))
    }
}

/// Un ítem con IVA solo puede ser venta o alquiler y servicio (la misma regla que la línea de
/// la factura y el `CHECK` de la BD); la tarifa va de 0 a 100. Que la tarifa esté vigente en el
/// catálogo se exige al facturar.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no cumple.
pub fn validar_iva_item(
    tarifa: Decimal,
    clasificacion: ClasificacionTributaria,
) -> Result<(), DominioError> {
    if tarifa < Decimal::ZERO || tarifa > Decimal::ONE_HUNDRED {
        return Err(invalido("tarifa_iva", "de 0 a 100"));
    }
    let gravable = matches!(
        clasificacion,
        ClasificacionTributaria::AlquilerServicio | ClasificacionTributaria::Venta
    );
    if tarifa.is_zero() || gravable {
        Ok(())
    } else {
        Err(invalido(
            "tarifa_iva",
            "solo la venta o el alquiler y servicio llevan IVA",
        ))
    }
}

/// Ambiente del servicio web de la DIAN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbienteDian {
    /// Pruebas: lo que se envía no tiene validez.
    Habilitacion,
    Produccion,
}

enum_texto!(AmbienteDian {
    Habilitacion => "habilitacion",
    Produccion => "produccion",
});

/// Estado de la habilitación de una entidad con el motor propio (Res. 000165 de 2023,
/// compilada en el título 5 de la Res. 000227 de 2025).
///
/// ```text
/// Registrado ──▶ EnPruebas ──▶ Habilitado ──▶ Produccion
///      ▲             │ ▲            │
///      └─────────────┘ └────────────┘   (cambiar el software vuelve a Registrado;
///                                         reenviar el set vuelve a EnPruebas)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoHabilitacion {
    /// SoftwareID, PIN y TestSetId registrados; falta enviar el set de pruebas.
    Registrado,
    /// Set de pruebas enviado; falta que la DIAN lo acepte.
    EnPruebas,
    /// La DIAN aceptó el set: se puede pasar a producción.
    Habilitado,
    /// Factura con validez.
    Produccion,
}

enum_texto!(EstadoHabilitacion {
    Registrado => "registrado",
    EnPruebas => "en_pruebas",
    Habilitado => "habilitado",
    Produccion => "produccion",
});

impl EstadoHabilitacion {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si la transición no está en el diagrama. En
    /// producción ya no se vuelve atrás.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoHabilitacion::{EnPruebas, Habilitado, Produccion, Registrado};
        let ok = matches!(
            (self, hacia),
            (Registrado | EnPruebas | Habilitado, Registrado)
                | (Registrado | EnPruebas | Habilitado, EnPruebas)
                | (EnPruebas, Habilitado)
                | (Habilitado, Produccion)
        );
        if ok {
            Ok(hacia)
        } else {
            Err(DominioError::TransicionInvalida {
                entidad: "habilitación de facturación",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            })
        }
    }

    /// Ambiente del servicio web que corresponde al estado.
    pub const fn ambiente(self) -> AmbienteDian {
        match self {
            Self::Produccion => AmbienteDian::Produccion,
            _ => AmbienteDian::Habilitacion,
        }
    }
}

/// Resultado de un documento del set de pruebas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoEnvioSet {
    Enviado,
    Aceptado,
    Rechazado,
    /// La DIAN aún no tiene respuesta (se vuelve a consultar).
    SinRespuesta,
}

enum_texto!(EstadoEnvioSet {
    Enviado => "enviado",
    Aceptado => "aceptado",
    Rechazado => "rechazado",
    SinRespuesta => "sin_respuesta",
});

/// Días antes del vencimiento del certificado en que se avisa.
pub const DIAS_AVISO_CERTIFICADO: i64 = 30;

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    #[test]
    fn el_codigo_del_item_se_recorta_y_valida() {
        assert_eq!(validar_codigo_item(" SRV-01 ").as_deref(), Ok("SRV-01"));
        assert!(validar_codigo_item("").is_err());
        assert!(validar_codigo_item("con espacio").is_err());
        assert!(validar_codigo_item(&"x".repeat(31)).is_err());
    }

    #[test]
    fn la_unidad_es_un_codigo_dian() {
        assert_eq!(validar_unidad("94").as_deref(), Ok("94"));
        assert_eq!(validar_unidad("HUR").as_deref(), Ok("HUR"));
        assert!(validar_unidad("hur").is_err());
        assert!(validar_unidad("HORA").is_err());
    }

    #[test]
    fn solo_la_venta_o_el_servicio_llevan_iva() {
        assert!(validar_iva_item(dec!(19), ClasificacionTributaria::Venta).is_ok());
        assert!(validar_iva_item(dec!(19), ClasificacionTributaria::AlquilerServicio).is_ok());
        assert!(validar_iva_item(dec!(0), ClasificacionTributaria::Expensa).is_ok());
        assert!(validar_iva_item(dec!(19), ClasificacionTributaria::Expensa).is_err());
        assert!(validar_iva_item(dec!(101), ClasificacionTributaria::Venta).is_err());
    }

    #[test]
    fn la_habilitacion_avanza_en_orden_y_produccion_no_vuelve_atras() {
        use EstadoHabilitacion::{EnPruebas, Habilitado, Produccion, Registrado};
        assert_eq!(Registrado.transicionar(EnPruebas), Ok(EnPruebas));
        assert_eq!(EnPruebas.transicionar(Habilitado), Ok(Habilitado));
        assert_eq!(Habilitado.transicionar(Produccion), Ok(Produccion));
        assert_eq!(
            EnPruebas.transicionar(EnPruebas),
            Ok(EnPruebas),
            "reenviar el set"
        );
        assert!(
            Registrado.transicionar(Habilitado).is_err(),
            "sin set no se habilita"
        );
        assert!(Registrado.transicionar(Produccion).is_err());
        assert!(Produccion.transicionar(Registrado).is_err());
        assert_eq!(Produccion.ambiente(), AmbienteDian::Produccion);
        assert_eq!(Habilitado.ambiente(), AmbienteDian::Habilitacion);
    }

    #[test]
    fn medio_de_pago_ida_y_vuelta() {
        for m in MedioPagoManual::TODOS {
            assert_eq!(MedioPagoManual::try_from(m.as_str()), Ok(*m));
        }
    }
}
