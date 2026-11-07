//! Protección de datos personales (Ley 1581 de 2012, Decreto 1377 de 2013, hoy compilado en el
//! Decreto 1074 de 2015).
//!
//! Cada copropiedad es **responsable** del tratamiento y la plataforma es **encargada**. La
//! copropiedad publica su política de tratamiento; cada titular autoriza finalidades concretas
//! (art. 9) y puede consultar (art. 14) o reclamar (art. 15) en plazos de días hábiles.

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::calendario::sumar_dias_habiles;

/// Para qué se tratan los datos. La autorización es por finalidad: aceptar una no implica
/// aceptar las demás (por ejemplo, la publicidad siempre es aparte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalidadTratamiento {
    /// Administración de la copropiedad: registro de propietarios y tenedores, expensas,
    /// convocatorias y asambleas.
    Administracion,
    /// Avisos y comunicaciones por WhatsApp, correo o notificaciones de la app.
    Comunicaciones,
    /// Registro de ingresos y salidas, visitantes y vehículos.
    ControlAcceso,
    /// Ofertas de comercios locales (plan 08). Nunca se transfieren datos al anunciante.
    Publicidad,
}

enum_texto!(FinalidadTratamiento {
    Administracion => "administracion",
    Comunicaciones => "comunicaciones",
    ControlAcceso => "control_acceso",
    Publicidad => "publicidad",
});

/// Medio por el que el titular otorgó la autorización. Debe poder probarse (Decreto 1377,
/// art. 7): el medio, la fecha y, si existe, la evidencia (archivo firmado).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MedioAutorizacion {
    Web,
    App,
    WhatsApp,
    /// Formato en papel firmado; su copia escaneada va como evidencia.
    Escrito,
}

enum_texto!(MedioAutorizacion {
    Web => "web",
    App => "app",
    WhatsApp => "whatsapp",
    Escrito => "escrito",
});

/// Solicitud del titular sobre sus datos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoSolicitudTitular {
    /// Conocer qué datos suyos se tratan (art. 14).
    Consulta,
    /// Reclamos (art. 15): actualizar, corregir, suprimir datos o revocar la autorización.
    Actualizacion,
    Rectificacion,
    Supresion,
    Revocatoria,
}

enum_texto!(TipoSolicitudTitular {
    Consulta => "consulta",
    Actualizacion => "actualizacion",
    Rectificacion => "rectificacion",
    Supresion => "supresion",
    Revocatoria => "revocatoria",
});

impl TipoSolicitudTitular {
    /// ¿Es un reclamo (art. 15) y no una consulta (art. 14)?
    pub const fn es_reclamo(self) -> bool {
        !matches!(self, Self::Consulta)
    }

    /// Plazo para responder: 10 días hábiles las consultas, 15 los reclamos.
    pub const fn plazo_dias_habiles(self) -> u32 {
        if self.es_reclamo() { 15 } else { 10 }
    }

    /// Prórroga máxima, avisando al titular antes del vencimiento: 5 días hábiles las
    /// consultas, 8 los reclamos.
    pub const fn prorroga_dias_habiles(self) -> u32 {
        if self.es_reclamo() { 8 } else { 5 }
    }

    /// Último día para responder una solicitud radicada el día `radicada` (hora de Colombia).
    /// El día de radicación no cuenta.
    pub fn vencimiento(self, radicada: NaiveDate) -> Option<NaiveDate> {
        sumar_dias_habiles(radicada, self.plazo_dias_habiles())
    }

    /// Último día si se usa la prórroga.
    pub fn vencimiento_con_prorroga(self, radicada: NaiveDate) -> Option<NaiveDate> {
        sumar_dias_habiles(
            radicada,
            self.plazo_dias_habiles() + self.prorroga_dias_habiles(),
        )
    }
}

/// Fecha civil en Colombia (UTC−5, sin horario de verano) de un instante. Los plazos legales
/// se cuentan en días de Colombia, no de UTC: una solicitud de las 9 p. m. es de ese día.
pub fn fecha_en_colombia(instante: DateTime<Utc>) -> NaiveDate {
    let colombia = FixedOffset::west_opt(5 * 3600).expect("UTC−5 es un desfase válido");
    instante.with_timezone(&colombia).date_naive()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha válida")
    }

    #[test]
    fn consulta_vence_en_10_dias_habiles_y_reclamo_en_15() {
        // Lunes 2 de noviembre de 2026 es festivo (Todos los Santos trasladado) y el 16 también
        // (Independencia de Cartagena trasladada).
        let radicada = fecha(2026, 10, 30); // viernes
        assert_eq!(
            TipoSolicitudTitular::Consulta.vencimiento(radicada),
            Some(fecha(2026, 11, 17))
        );
        assert_eq!(
            TipoSolicitudTitular::Supresion.vencimiento(radicada),
            Some(fecha(2026, 11, 24))
        );
    }

    #[test]
    fn prorroga_suma_5_dias_a_consultas_y_8_a_reclamos() {
        let radicada = fecha(2026, 10, 30);
        assert_eq!(
            TipoSolicitudTitular::Consulta.vencimiento_con_prorroga(radicada),
            Some(fecha(2026, 11, 24))
        );
        assert_eq!(
            TipoSolicitudTitular::Revocatoria.vencimiento_con_prorroga(radicada),
            Some(fecha(2026, 12, 4))
        );
    }

    #[test]
    fn solo_la_consulta_no_es_reclamo() {
        for tipo in TipoSolicitudTitular::TODOS {
            assert_eq!(tipo.es_reclamo(), *tipo != TipoSolicitudTitular::Consulta);
        }
    }

    #[test]
    fn la_fecha_de_radicacion_es_la_de_colombia() {
        // 2 a. m. UTC del 31 de octubre = 9 p. m. del 30 en Colombia.
        let instante = Utc
            .with_ymd_and_hms(2026, 10, 31, 2, 0, 0)
            .single()
            .expect("fecha válida");
        assert_eq!(fecha_en_colombia(instante), fecha(2026, 10, 30));
    }

    #[test]
    fn textos_persistidos() {
        assert_eq!(
            FinalidadTratamiento::ControlAcceso.as_str(),
            "control_acceso"
        );
        assert_eq!(MedioAutorizacion::WhatsApp.as_str(), "whatsapp");
        assert_eq!(
            TipoSolicitudTitular::try_from("supresion"),
            Ok(TipoSolicitudTitular::Supresion)
        );
    }
}
