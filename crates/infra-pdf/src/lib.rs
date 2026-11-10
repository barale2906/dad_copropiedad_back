//! Generación de PDF con Typst (como librería): el acta de la asamblea y la hoja de rótulos de
//! código de barras. Implementa [`application::puertos::GeneradorActas`]. También genera el QR
//! de las visitas en PNG ([`GeneradorQrPng`], puerto `GeneradorImagenQr`).
//!
//! Lineamientos:
//! - Las plantillas Typst están versionadas en `plantillas/` y se embeben al compilar.
//! - El adaptador solo inyecta datos **ya calculados** ([`DatosActa`], [`HojaRotulos`]) como
//!   un archivo virtual `datos.json`: no decide quórum ni resultados.
//! - Fuentes embebidas (`typst-assets`): el PDF sale igual en cualquier servidor, sin depender
//!   de las fuentes instaladas.
//! - Compilar es trabajo de CPU: se hace en `spawn_blocking`, nunca en la ruta de votación.

mod code128;
mod mundo;
mod qr;

use application::{
    AppError, AppResult,
    puertos::{
        GeneradorActas,
        actas::{DatosActa, HojaRotulos},
    },
};
use serde_json::json;

pub use code128::modulos as code128;
pub use qr::GeneradorQrPng;

const PLANTILLA_ACTA: &str = include_str!("../plantillas/acta.typ");
const PLANTILLA_ROTULOS: &str = include_str!("../plantillas/rotulos.typ");

/// Generador de PDF con Typst.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeneradorTypst;

async fn compilar(plantilla: &'static str, datos: serde_json::Value) -> AppResult<Vec<u8>> {
    let datos = serde_json::to_vec(&datos).map_err(AppError::infra)?;
    tokio::task::spawn_blocking(move || mundo::compilar(plantilla, datos))
        .await
        .map_err(AppError::infra)?
}

impl GeneradorActas for GeneradorTypst {
    async fn generar(&self, datos: &DatosActa) -> AppResult<Vec<u8>> {
        compilar(
            PLANTILLA_ACTA,
            serde_json::to_value(datos).map_err(AppError::infra)?,
        )
        .await
    }

    async fn rotulos(&self, hoja: &HojaRotulos) -> AppResult<Vec<u8>> {
        let rotulos: Vec<_> = hoja
            .codigos
            .iter()
            .map(|c| {
                code128::modulos(c)
                    .map(|m| json!({ "codigo": c, "modulos": m }))
                    .ok_or_else(|| {
                        AppError::Validacion(format!("«{c}» no se puede codificar en Code 128"))
                    })
            })
            .collect::<AppResult<_>>()?;
        compilar(
            PLANTILLA_ROTULOS,
            json!({ "titulo": hoja.titulo, "rotulos": rotulos }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use application::puertos::actas::{AsistenteActa, PreguntaActa, QuorumActa, VotoActa};
    use rust_decimal::Decimal;

    use super::*;

    fn escrutinio() -> domain::Escrutinio {
        use domain::{
            BaseComputo, Conteo, EstadoPregunta, OpcionId, OpcionPregunta, Pregunta, PreguntaId,
            ReglaMayoria, ReunionId, TipoPregunta, TipoVoto, VotosOpcion,
        };
        let opciones: Vec<OpcionPregunta> = ["Sí", "No"]
            .iter()
            .zip(1..)
            .map(|(t, orden)| OpcionPregunta {
                id: OpcionId::nuevo(),
                orden,
                texto: (*t).into(),
                en_blanco: false,
            })
            .collect();
        let votos = vec![VotosOpcion {
            opcion_id: opciones[0].id,
            votos: Conteo::new(Decimal::new(30, 2), 1),
        }];
        let p = Pregunta {
            id: PreguntaId::nuevo(),
            reunion_id: ReunionId::nuevo(),
            enunciado: "¿Aprueba?".into(),
            tipo: TipoPregunta::SiNo,
            tipo_voto: TipoVoto::Nominal,
            base_computo: BaseComputo::Coeficiente,
            regla_mayoria: ReglaMayoria::Simple,
            max_selecciones: 1,
            cupos: None,
            cargo: None,
            estado: EstadoPregunta::Cerrada,
            opciones,
        };
        let hab = Conteo::new(Decimal::new(55, 2), 2);
        domain::Escrutinio::calcular(&p, hab, Conteo::new(Decimal::ONE, 5), hab, &votos, |_| None)
    }

    fn datos() -> DatosActa {
        DatosActa {
            copropiedad: "Conjunto Alameda".into(),
            nit: Some("900123456-7".into()),
            titulo: "Asamblea ordinaria 2027".into(),
            tipo: "ordinaria".into(),
            convocatoria: "primera".into(),
            modalidad: "presencial".into(),
            lugar: Some("Salón comunal".into()),
            fecha: "2027-03-15 19:00".into(),
            instalada: Some("2027-03-15 19:10".into()),
            cerrada: Some("2027-03-15 21:40".into()),
            orden_del_dia: vec!["1. Verificación del quórum".into(), "2. Presupuesto".into()],
            quorum: vec![QuorumActa {
                momento: "instalacion".into(),
                hora: "2027-03-15 19:10".into(),
                coeficiente: Decimal::new(55, 2),
                porcentaje_coeficiente: Decimal::new(55, 0),
                unidades: 2,
                unidades_total: 5,
                asistentes: 1,
            }],
            asistentes: vec![AsistenteActa {
                nombre: "Ana Pérez".into(),
                bienes: vec!["T1-101".into(), "T1-201".into()],
                coeficiente: Decimal::new(55, 2),
                poderes: vec!["T1-201 (pendiente)".into()],
                entrada: "2027-03-15 18:55".into(),
                salidas: vec!["2027-03-15 20:00 (se retiro)".into()],
            }],
            morosos: vec!["T1-301".into()],
            efecto_mora: "sin voto".into(),
            preguntas: vec![PreguntaActa {
                orden: 1,
                enunciado: "¿Aprueba el presupuesto 2027?".into(),
                tipo: "si no".into(),
                tipo_voto: "nominal".into(),
                abierta: Some("19:20".into()),
                cerrada: Some("19:25".into()),
                escrutinio: Some(escrutinio()),
                decision: "aprobada".into(),
                detalle: vec![VotoActa {
                    bien: "T1-101".into(),
                    coeficiente: Decimal::new(30, 2),
                    opcion: "Sí".into(),
                    canal: "whatsapp".into(),
                    hora: "19:21".into(),
                    apoderado: false,
                }],
                elegidos: vec!["Ana Pérez".into()],
            }],
            advertencias: vec!["Decisión por unidades".into()],
            generada: "2027-03-16 08:00".into(),
        }
    }

    #[tokio::test]
    async fn genera_el_acta_en_pdf() {
        let pdf = GeneradorTypst.generar(&datos()).await.expect("compila");
        assert!(pdf.starts_with(b"%PDF-"), "es un PDF");
        assert!(pdf.len() > 1000);
    }

    #[tokio::test]
    async fn genera_la_hoja_de_rotulos() {
        let hoja = HojaRotulos {
            titulo: "Conjunto Alameda".into(),
            codigos: (1..=30).map(|i| format!("R-{i:04}")).collect(),
        };
        let pdf = GeneradorTypst.rotulos(&hoja).await.expect("compila");
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[tokio::test]
    async fn rechaza_codigos_que_no_caben_en_code_128() {
        let hoja = HojaRotulos {
            titulo: "x".into(),
            codigos: vec!["Ñ".into()],
        };
        assert!(matches!(
            GeneradorTypst.rotulos(&hoja).await,
            Err(AppError::Validacion(_))
        ));
    }
}
