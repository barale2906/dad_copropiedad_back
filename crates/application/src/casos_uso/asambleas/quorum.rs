//! Quórum en vivo y tablero de las pantallas (WebSocket): por coeficiente y por unidades, con
//! los bienes de los asistentes **presentes** (los retirados salen) y sin los morosos cuando el
//! efecto de la mora es `sin_voto_ni_quorum`.

use super::{TxAsambleas, abrir, reunion_de, votaciones::escrutinio_de};
use crate::{
    AppError, AppResult, Contexto,
    puertos::{
        Reloj, RepositorioAsambleas, Transaccion, UnidadDeTrabajo,
        asambleas::{BienPresente, FotoQuorum, PreguntaGuardada, Reunion},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    Conteo, EfectoMora, EstadoPregunta, EstadoPunto, MomentoQuorum, Permiso, PreguntaId,
    QuorumReunion, ReunionId,
};

/// Bienes que cuentan para el quórum según el efecto de la mora.
pub(crate) fn cuentan_para_quorum(
    bienes: &[BienPresente],
    efecto: EfectoMora,
) -> Vec<BienPresente> {
    bienes
        .iter()
        .filter(|b| !b.moroso || efecto.cuenta_para_quorum())
        .copied()
        .collect()
}

/// Bienes que pueden votar según el efecto de la mora.
pub(crate) fn pueden_votar(bienes: &[BienPresente], efecto: EfectoMora) -> Vec<BienPresente> {
    bienes
        .iter()
        .filter(|b| !b.moroso || efecto.vota())
        .copied()
        .collect()
}

pub(crate) fn conteo(bienes: &[BienPresente]) -> Conteo {
    Conteo::new(
        bienes.iter().map(|b| b.coeficiente.valor()).sum(),
        u32::try_from(bienes.len()).unwrap_or(u32::MAX),
    )
}

/// Quórum actual de la reunión.
pub(crate) async fn calcular_quorum<T: TxAsambleas>(
    tx: &mut T,
    r: &Reunion,
) -> AppResult<QuorumReunion> {
    let padron = tx.padron(r.id).await?;
    if padron.is_empty() {
        return Err(AppError::Conflicto(
            "la reunión aún no tiene padrón: convóquela primero".into(),
        ));
    }
    let total = Conteo::new(
        padron.iter().map(|b| b.coeficiente.valor()).sum(),
        u32::try_from(padron.len()).unwrap_or(u32::MAX),
    );
    let presentes = cuentan_para_quorum(
        &tx.bienes_presentes(r.id).await?,
        r.configuracion.efecto_mora,
    );
    let mut asistentes: Vec<_> = presentes.iter().map(|b| b.asistente_id).collect();
    asistentes.sort();
    asistentes.dedup();
    let c = conteo(&presentes);
    Ok(QuorumReunion::new(
        c.coeficiente,
        total.coeficiente,
        c.unidades,
        total.unidades,
        u32::try_from(asistentes.len()).unwrap_or(u32::MAX),
    )?)
}

/// Guarda la foto del quórum (instalación, apertura y cierre de preguntas).
pub(crate) async fn tomar_foto<T: TxAsambleas>(
    tx: &mut T,
    r: &Reunion,
    pregunta: Option<PreguntaId>,
    momento: MomentoQuorum,
    quorum: QuorumReunion,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    tx.insertar_foto(
        r.id,
        &FotoQuorum {
            pregunta_id: pregunta,
            momento,
            quorum,
            tomada_en: ahora,
        },
    )
    .await
}

/// Quórum actual.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `asambleas.ver`; [`crate::AppError::NoEncontrado`].
pub async fn quorum_actual<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<QuorumReunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, id).await?;
    calcular_quorum(&mut tx, &r).await
}

/// Toma una foto manual del quórum (p. ej. antes de un punto importante).
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `asambleas.conducir`; [`crate::AppError::NoEncontrado`].
pub async fn foto_manual<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<QuorumReunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, id).await?;
    let q = calcular_quorum(&mut tx, &r).await?;
    tomar_foto(&mut tx, &r, None, MomentoQuorum::Manual, q, reloj.ahora()).await?;
    tx.confirmar().await?;
    Ok(q)
}

/// Pregunta abierta (o recién cerrada) con su escrutinio en vivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreguntaEnVivo {
    pub pregunta: PreguntaGuardada,
    pub escrutinio: domain::Escrutinio,
}

/// Lo que muestran las pantallas en vivo (conductor, proyector, mesa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableroEnVivo {
    pub reunion: Reunion,
    pub quorum: QuorumReunion,
    pub hay_quorum: bool,
    pub punto_en_curso: Option<String>,
    pub preguntas_abiertas: Vec<PreguntaEnVivo>,
    /// Última pregunta cerrada (para mostrar su resultado).
    pub ultima_cerrada: Option<PreguntaEnVivo>,
    pub generado_en: DateTime<Utc>,
}

/// Tablero en vivo: quórum, punto en curso y escrutinio de las preguntas abiertas. Se
/// recalcula desde la BD (la única verdad) cuando algo cambia, a lo sumo cada ~500 ms.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `asambleas.ver`; [`crate::AppError::NoEncontrado`].
pub async fn tablero_en_vivo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<TableroEnVivo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, id).await?;
    let quorum = calcular_quorum(&mut tx, &r).await?;
    let punto_en_curso = tx
        .puntos(id)
        .await?
        .into_iter()
        .find(|p| p.estado == EstadoPunto::EnCurso)
        .map(|p| p.titulo);
    let mut preguntas_abiertas = Vec::new();
    let mut ultima_cerrada: Option<PreguntaGuardada> = None;
    for p in tx.preguntas(id).await? {
        match p.pregunta.estado {
            EstadoPregunta::Abierta => {
                let escrutinio = escrutinio_de(&mut tx, &p).await?;
                preguntas_abiertas.push(PreguntaEnVivo {
                    pregunta: p,
                    escrutinio,
                });
            }
            EstadoPregunta::Cerrada
                if ultima_cerrada
                    .as_ref()
                    .is_none_or(|u| u.cerrada_en < p.cerrada_en) =>
            {
                ultima_cerrada = Some(p);
            }
            _ => {}
        }
    }
    let ultima_cerrada = match ultima_cerrada {
        Some(p) => {
            let escrutinio = match &p.resultado {
                Some(e) => e.clone(),
                None => escrutinio_de(&mut tx, &p).await?,
            };
            Some(PreguntaEnVivo {
                pregunta: p,
                escrutinio,
            })
        }
        None => None,
    };
    Ok(TableroEnVivo {
        hay_quorum: quorum.hay_quorum(r.convocatoria, r.configuracion.base_computo),
        reunion: r,
        quorum,
        punto_en_curso,
        preguntas_abiertas,
        ultima_cerrada,
        generado_en: reloj.ahora(),
    })
}
