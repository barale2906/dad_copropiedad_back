//! Reportes de la asamblea: asistencia, votos por pregunta, participación por canal y costo
//! estimado de WhatsApp (se cobra dentro del valor de la asamblea, decisión del 2026-11-08).

use chrono::{DateTime, Utc};
use domain::{Canal, Permiso, PreguntaId, ReunionId, TipoVoto};
use rust_decimal::Decimal;

use super::{TxAsambleas, abrir, reunion_de, votaciones::detalle_votos, zona_de};
use crate::{
    AppResult, Contexto,
    puertos::{
        Reloj, RepositorioAsambleas, UnidadDeTrabajo,
        asambleas::{MensajesLinea, ParticipacionActa},
    },
};

/// Fila del reporte de asistencia (una por bien representado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaAsistencia {
    pub asistente: String,
    pub bien: String,
    pub coeficiente: Decimal,
    pub calidad: String,
    pub metodo: String,
    pub moroso: bool,
    pub entrada: Option<DateTime<Utc>>,
    pub salida: Option<DateTime<Utc>>,
    pub motivo_salida: Option<String>,
    pub whatsapp_confirmado: bool,
}

/// Asistencia: un asistente por bien, con su primera entrada y su última salida.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `asambleas.ver`; [`crate::AppError::NoEncontrado`].
pub async fn reporte_asistencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Vec<FilaAsistencia>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    let presencias = tx.presencias(reunion).await?;
    let mut filas = Vec::new();
    for a in tx.asistentes(reunion).await? {
        let propias: Vec<_> = presencias
            .iter()
            .filter(|p| p.asistente_id == a.id)
            .collect();
        let entrada = propias.iter().map(|p| p.entrada_en).min();
        let ultima = propias.iter().max_by_key(|p| p.entrada_en);
        for b in &a.bienes {
            filas.push(FilaAsistencia {
                asistente: a.nombre.clone(),
                bien: b.codigo.clone(),
                coeficiente: b.coeficiente.valor(),
                calidad: b.calidad.as_str().into(),
                metodo: a.metodo.as_str().into(),
                moroso: b.moroso,
                entrada,
                salida: ultima.and_then(|p| p.salida_en),
                motivo_salida: ultima
                    .and_then(|p| p.motivo_salida)
                    .map(|m| m.as_str().into()),
                whatsapp_confirmado: a.whatsapp_confirmado_en.is_some(),
            });
        }
    }
    filas.sort_by(|x, y| x.bien.cmp(&y.bien));
    Ok(filas)
}

/// Votos de una pregunta: quién votó por cada bien y, en nominales (o con permiso en
/// reservadas), qué opción eligió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaVoto {
    pub participacion: ParticipacionActa,
    /// `None` en votaciones reservadas sin permiso de detalle.
    pub opcion: Option<String>,
}

/// # Errors
///
/// [`crate::AppError::SinPermiso`]; [`crate::AppError::NoEncontrado`].
pub async fn reporte_votos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    pregunta: PreguntaId,
) -> AppResult<Vec<FilaVoto>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let (p, participaciones) = {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        let p = tx
            .pregunta(pregunta)
            .await?
            .filter(|p| p.pregunta.reunion_id == reunion)
            .ok_or(crate::AppError::NoEncontrado("pregunta"))?;
        let participaciones = tx.participaciones(pregunta).await?;
        (p, participaciones)
    };
    let con_detalle = p.pregunta.tipo_voto == TipoVoto::Nominal
        || ctx.exigir(Permiso::AsambleasVotosVerDetalle).is_ok();
    let detalle = if con_detalle {
        detalle_votos(unidad, reloj, ctx, reunion, pregunta).await?
    } else {
        Vec::new()
    };
    Ok(participaciones
        .into_iter()
        .map(|part| {
            let opciones: Vec<_> = detalle
                .iter()
                .filter(|d| d.bien_privado_id == part.bien_privado_id)
                .filter_map(|d| p.pregunta.opcion(d.opcion_id).map(|o| o.texto.clone()))
                .collect();
            FilaVoto {
                opcion: con_detalle.then(|| opciones.join(", ")),
                participacion: part,
            }
        })
        .collect())
}

/// Participación por canal de una pregunta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipacionCanal {
    pub pregunta_id: PreguntaId,
    pub orden: u16,
    pub enunciado: String,
    pub por_canal: Vec<(Canal, u32)>,
    pub primero: Option<DateTime<Utc>>,
    pub ultimo: Option<DateTime<Utc>>,
}

/// # Errors
///
/// [`crate::AppError::SinPermiso`]; [`crate::AppError::NoEncontrado`].
pub async fn participacion_por_canal<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Vec<ParticipacionCanal>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    let mut filas = Vec::new();
    for p in tx.preguntas(reunion).await? {
        let parts = tx.participaciones(p.pregunta.id).await?;
        let por_canal = Canal::TODOS
            .iter()
            .map(|c| {
                (
                    *c,
                    u32::try_from(parts.iter().filter(|x| x.canal == *c).count())
                        .unwrap_or(u32::MAX),
                )
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        filas.push(ParticipacionCanal {
            pregunta_id: p.pregunta.id,
            orden: p.orden,
            enunciado: p.pregunta.enunciado.clone(),
            por_canal,
            primero: parts.iter().map(|x| x.registrado_en).min(),
            ultimo: parts.iter().map(|x| x.registrado_en).max(),
        });
    }
    Ok(filas)
}

/// Costo estimado de WhatsApp de la asamblea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostoWhatsapp {
    pub moneda: String,
    pub plantillas: u64,
    pub servicio: u64,
    pub fallidos: u64,
    pub tarifa_utilidad: Decimal,
    pub tarifa_servicio: Decimal,
    /// Sin descontar la franquicia gratuita de cada línea (depende del uso de todo el mes).
    pub valor_estimado: Decimal,
    pub por_linea: Vec<MensajesLinea>,
}

/// Mensajes enviados por la asamblea (plantillas y servicio) valorizados con la tarifa
/// vigente a la fecha de la reunión.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `asambleas.convocar`; [`crate::AppError::NoEncontrado`].
pub async fn costo_whatsapp<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    moneda: &str,
) -> AppResult<CostoWhatsapp>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    let tz = zona_de(&mut tx).await?;
    let fecha = r.fecha_programada.with_timezone(&tz).date_naive();
    let tarifas = tx.tarifas_whatsapp("CO", moneda, fecha).await?;
    let tarifa = |c: &str| {
        tarifas
            .iter()
            .find(|(cat, _)| cat == c)
            .map_or(Decimal::ZERO, |(_, v)| *v)
    };
    let por_linea = tx.mensajes_whatsapp(reunion).await?;
    let plantillas: u64 = por_linea
        .iter()
        .filter(|l| l.es_plantilla)
        .map(|l| l.enviados)
        .sum();
    let servicio: u64 = por_linea
        .iter()
        .filter(|l| !l.es_plantilla)
        .map(|l| l.enviados)
        .sum();
    let (tu, ts) = (tarifa("utilidad"), tarifa("servicio"));
    Ok(CostoWhatsapp {
        moneda: moneda.to_owned(),
        plantillas,
        servicio,
        fallidos: por_linea.iter().map(|l| l.fallidos).sum(),
        tarifa_utilidad: tu,
        tarifa_servicio: ts,
        valor_estimado: (Decimal::from(plantillas) * tu + Decimal::from(servicio) * ts).round_dp(2),
        por_linea,
    })
}
