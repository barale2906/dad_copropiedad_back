//! DTOs compartidos del control de acceso.

use application::puertos::acceso::{
    Correspondencia, EntregaCorrespondencia, EventoAcceso, EventoVisita, TitularAcceso, Visita,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use domain::{PersonaId, VehiculoId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// Decodifica un archivo enviado en Base64 (estándar, con o sin prefijo `data:…;base64,`).
///
/// # Errors
///
/// [`ApiError::PeticionInvalida`] si no es Base64 válido.
pub fn bytes_base64(campo: &str, v: Option<&str>) -> ApiResult<Option<Vec<u8>>> {
    let Some(texto) = v.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let datos = texto.split_once(";base64,").map_or(texto, |(_, d)| d);
    STANDARD
        .decode(datos)
        .map(Some)
        .map_err(|_| ApiError::PeticionInvalida(format!("`{campo}` no es Base64 válido")))
}

/// Persona **o** vehículo (exactamente uno).
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TitularDto {
    pub persona_id: Option<Uuid>,
    pub vehiculo_id: Option<Uuid>,
}

impl TitularDto {
    /// # Errors
    ///
    /// [`ApiError::PeticionInvalida`] si no trae exactamente uno.
    pub fn validar(self) -> ApiResult<TitularAcceso> {
        match (self.persona_id, self.vehiculo_id) {
            (Some(p), None) => Ok(TitularAcceso::Persona(PersonaId::from_uuid(p))),
            (None, Some(v)) => Ok(TitularAcceso::Vehiculo(VehiculoId::from_uuid(v))),
            _ => Err(ApiError::PeticionInvalida(
                "indique `persona_id` o `vehiculo_id` (uno solo)".into(),
            )),
        }
    }
}

impl From<TitularAcceso> for TitularDto {
    fn from(t: TitularAcceso) -> Self {
        let (p, v) = t.columnas();
        Self {
            persona_id: p.map(PersonaId::as_uuid),
            vehiculo_id: v.map(VehiculoId::as_uuid),
        }
    }
}

/// Visita.
#[derive(Debug, Serialize, ToSchema)]
pub struct VisitaRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    /// `anuncio_porteria` o `preautorizada`.
    #[schema(value_type = crate::esquemas::OrigenVisita)]
    pub origen: String,
    /// `visita`, `domicilio`, `proveedor`, `trasteo` o `servicio_domestico`.
    #[schema(value_type = crate::esquemas::TipoVisita)]
    pub tipo: String,
    /// `anunciada`, `requiere_llamada`, `autorizada`, `rechazada`, `sin_respuesta`, `ingresada`,
    /// `finalizada`, `vencida` o `cancelada`.
    #[schema(value_type = crate::esquemas::EstadoVisita)]
    pub estado: String,
    pub visitante_nombre: String,
    pub visitante_documento: Option<String>,
    pub placa: Option<String>,
    pub observacion: Option<String>,
    pub decidida_por: Option<Uuid>,
    /// `app`, `web`, `whatsapp` o `llamada`.
    pub canal_decision: Option<String>,
    pub ventana_desde: Option<DateTime<Utc>>,
    pub ventana_hasta: Option<DateTime<Utc>>,
    pub usos_permitidos: u16,
    pub usos: u16,
    /// Avisos enviados (1 = primer respondiente, 2 = todos los residentes).
    pub avisos_enviados: u32,
    pub creada_en: DateTime<Utc>,
    pub decidida_en: Option<DateTime<Utc>>,
    pub ingreso_en: Option<DateTime<Utc>>,
    pub salida_en: Option<DateTime<Utc>>,
}

impl From<Visita> for VisitaRespuesta {
    fn from(v: Visita) -> Self {
        Self {
            id: v.id.as_uuid(),
            bien_privado_id: v.bien_privado_id.as_uuid(),
            origen: v.origen.as_str().into(),
            tipo: v.tipo.as_str().into(),
            estado: v.estado.as_str().into(),
            visitante_nombre: v.visitante_nombre,
            visitante_documento: v.visitante_documento.map(|d| d.as_str().to_owned()),
            placa: v.placa.map(|p| p.as_str().to_owned()),
            observacion: v.observacion,
            decidida_por: v.decidida_por.map(PersonaId::as_uuid),
            canal_decision: v.canal_decision.map(|c| c.as_str().into()),
            ventana_desde: v.ventana_desde,
            ventana_hasta: v.ventana_hasta,
            usos_permitidos: v.usos_permitidos,
            usos: v.usos,
            avisos_enviados: v.avisos_enviados,
            creada_en: v.creada_en,
            decidida_en: v.decidida_en,
            ingreso_en: v.ingreso_en,
            salida_en: v.salida_en,
        }
    }
}

/// Hecho del historial de una visita.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoVisitaRespuesta {
    #[schema(value_type = crate::esquemas::TipoEventoVisita)]
    pub tipo: String,
    pub canal: Option<String>,
    pub persona_id: Option<Uuid>,
    pub cuenta_id: Option<Uuid>,
    pub nivel: Option<u32>,
    pub destinatarios: Option<u32>,
    pub ocurrido_en: DateTime<Utc>,
}

impl From<EventoVisita> for EventoVisitaRespuesta {
    fn from(e: EventoVisita) -> Self {
        Self {
            tipo: e.tipo.as_str().into(),
            canal: e.canal.map(|c| c.as_str().into()),
            persona_id: e.persona_id.map(PersonaId::as_uuid),
            cuenta_id: e.cuenta_id.map(|c| c.as_uuid()),
            nivel: e.nivel,
            destinatarios: e.destinatarios,
            ocurrido_en: e.ocurrido_en,
        }
    }
}

/// Correspondencia.
#[derive(Debug, Serialize, ToSchema)]
pub struct CorrespondenciaRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    /// `paquete`, `sobre`, `recibo` u `otro`.
    #[schema(value_type = crate::esquemas::TipoCorrespondencia)]
    pub tipo: String,
    /// `recibida`, `entregada` o `devuelta`.
    #[schema(value_type = crate::esquemas::EstadoCorrespondencia)]
    pub estado: String,
    pub remitente: Option<String>,
    pub guia: Option<String>,
    pub ubicacion: Option<String>,
    pub foto_archivo_id: Option<Uuid>,
    pub recibida_en: DateTime<Utc>,
    pub avisos_enviados: u32,
    pub siguiente_aviso_en: Option<DateTime<Utc>>,
    pub alerta_administracion_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub motivo_devolucion: Option<String>,
}

impl From<Correspondencia> for CorrespondenciaRespuesta {
    fn from(c: Correspondencia) -> Self {
        Self {
            id: c.id.as_uuid(),
            bien_privado_id: c.bien_privado_id.as_uuid(),
            tipo: c.tipo.as_str().into(),
            estado: c.estado.as_str().into(),
            remitente: c.remitente,
            guia: c.guia,
            ubicacion: c.ubicacion,
            foto_archivo_id: c.foto_archivo_id.map(|a| a.as_uuid()),
            recibida_en: c.recibida_en,
            avisos_enviados: c.avisos_enviados,
            siguiente_aviso_en: c.siguiente_aviso_en,
            alerta_administracion_en: c.alerta_administracion_en,
            cerrada_en: c.cerrada_en,
            motivo_devolucion: c.motivo_devolucion,
        }
    }
}

/// Entrega de una correspondencia. El documento del tercero solo lo ve la portería.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntregaCorrespondenciaRespuesta {
    pub persona_id: Option<Uuid>,
    pub tercero_nombre: Option<String>,
    pub tercero_documento: Option<String>,
    pub firma_archivo_id: Option<Uuid>,
    pub foto_archivo_id: Option<Uuid>,
    pub entregada_en: DateTime<Utc>,
    /// Hasta cuándo se guardan la firma y la foto.
    pub soportes_hasta: DateTime<Utc>,
}

impl From<EntregaCorrespondencia> for EntregaCorrespondenciaRespuesta {
    fn from(e: EntregaCorrespondencia) -> Self {
        Self {
            persona_id: e.persona_id.map(PersonaId::as_uuid),
            tercero_nombre: e.tercero_nombre,
            tercero_documento: e.tercero_documento.map(|d| d.as_str().to_owned()),
            firma_archivo_id: e.firma_archivo_id.map(|a| a.as_uuid()),
            foto_archivo_id: e.foto_archivo_id.map(|a| a.as_uuid()),
            entregada_en: e.entregada_en,
            soportes_hasta: e.soportes_hasta,
        }
    }
}

/// Evento de acceso de la bitácora.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoAccesoRespuesta {
    pub id: Uuid,
    pub punto_acceso_id: Option<Uuid>,
    pub dispositivo_id: Option<Uuid>,
    pub puente_id: Option<Uuid>,
    pub credencial_id: Option<Uuid>,
    pub visita_id: Option<Uuid>,
    /// `entrada`, `salida` o `ambos`.
    #[schema(value_type = crate::esquemas::SentidoAcceso)]
    pub sentido: String,
    /// `permitido`, `denegado`, `forzado` o `manual`.
    #[schema(value_type = crate::esquemas::ResultadoAcceso)]
    pub resultado: String,
    pub motivo: Option<String>,
    pub placa: Option<String>,
    pub registrado_por: Option<Uuid>,
    pub ocurrido_en: DateTime<Utc>,
}

impl From<EventoAcceso> for EventoAccesoRespuesta {
    fn from(e: EventoAcceso) -> Self {
        Self {
            id: e.id.as_uuid(),
            punto_acceso_id: e.punto_acceso_id.map(|x| x.as_uuid()),
            dispositivo_id: e.dispositivo_id.map(|x| x.as_uuid()),
            puente_id: e.puente_id.map(|x| x.as_uuid()),
            credencial_id: e.credencial_id.map(|x| x.as_uuid()),
            visita_id: e.visita_id.map(|x| x.as_uuid()),
            sentido: e.sentido.as_str().into(),
            resultado: e.resultado.as_str().into(),
            motivo: e.motivo,
            placa: e.placa.map(|p| p.as_str().to_owned()),
            registrado_por: e.registrado_por.map(|x| x.as_uuid()),
            ocurrido_en: e.ocurrido_en,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_con_y_sin_prefijo() {
        assert_eq!(
            bytes_base64("x", Some("aG9sYQ==")).expect("válido"),
            Some(b"hola".to_vec())
        );
        assert_eq!(
            bytes_base64("x", Some("data:image/png;base64,aG9sYQ==")).expect("válido"),
            Some(b"hola".to_vec())
        );
        assert_eq!(bytes_base64("x", Some("  ")).expect("vacío"), None);
        assert!(bytes_base64("x", Some("@@@")).is_err());
    }

    #[test]
    fn titular_exige_uno_solo() {
        assert!(TitularDto::default().validar().is_err());
        assert!(
            TitularDto {
                persona_id: Some(Uuid::nil()),
                vehiculo_id: Some(Uuid::nil())
            }
            .validar()
            .is_err()
        );
    }
}
