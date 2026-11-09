//! Acta de la reunión (Ley 675, art. 47): se genera en PDF con todos los datos calculados
//! (asistencia con coeficientes, quórum por momento, cada pregunta con su resultado por
//! coeficiente y por unidades, detalle nominal, morosos y su efecto, poderes y retiros). Sin
//! firma electrónica: se suben las firmas escaneadas, la comisión la aprueba y se publica en el
//! libro de actas. También genera la hoja de rótulos de código de barras.

use domain::{
    ArchivoId, ClasificacionArchivo, EstadoActa, EstadoPregunta, EstadoReunion, NombreArchivo,
    Permiso, ReunionId, TipoArchivo, TipoVoto, reserva::Tz,
};

use rust_decimal::prelude::ToPrimitive;

use super::{
    ACCION_ACTA, TxAsambleas, abrir, reunion_bloqueada, reunion_de,
    votaciones::{elegidos_de, escrutinio_de, texto_decision},
    zona_de,
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    puertos::{
        AlmacenArchivos, GeneradorActas, RegistroAuditoria, Reloj, RepositorioArchivos,
        RepositorioAsambleas, RepositorioCopropiedad, Transaccion, UnidadDeTrabajo,
        actas::{
            AsistenteActa, DatosActa, HojaRotulos, PreguntaActa, QuorumActa, VotoActa, hora_local,
        },
        asambleas::Reunion,
    },
};

/// Máximo de rótulos por hoja.
pub const MAX_ROTULOS: u32 = 1000;

fn hora(t: chrono::DateTime<chrono::Utc>, tz: Tz) -> String {
    hora_local(t, tz)
}

/// Arma los datos del acta (todo calculado aquí; el generador solo da formato).
pub(crate) async fn datos_acta<T: TxAsambleas>(
    tx: &mut T,
    r: &Reunion,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<DatosActa> {
    let tz = zona_de(tx).await?;
    let ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    let padron = tx.padron(r.id).await?;
    let codigo = |id| {
        padron
            .iter()
            .find(|b| b.bien_privado_id == id)
            .map(|b| b.codigo.clone())
            .unwrap_or_default()
    };
    let quorum = tx
        .fotos(r.id)
        .await?
        .into_iter()
        .map(|f| QuorumActa {
            momento: f.momento.as_str().into(),
            hora: hora(f.tomada_en, tz),
            coeficiente: f.quorum.coeficientes.representados,
            porcentaje_coeficiente: (f.quorum.coeficientes.fraccion()
                * rust_decimal::Decimal::ONE_HUNDRED)
                .round_dp(4),
            unidades: f.quorum.unidades.representados.to_u32().unwrap_or(0),
            unidades_total: f.quorum.unidades.total.to_u32().unwrap_or(0),
            asistentes: f.quorum.asistentes,
        })
        .collect();
    let presencias = tx.presencias(r.id).await?;
    let poderes = tx.poderes(r.id).await?;
    let mut asistentes = Vec::new();
    for a in tx.asistentes(r.id).await? {
        let propias: Vec<_> = presencias
            .iter()
            .filter(|p| p.asistente_id == a.id)
            .collect();
        asistentes.push(AsistenteActa {
            nombre: a.nombre.clone(),
            bienes: a.bienes.iter().map(|b| b.codigo.clone()).collect(),
            coeficiente: a.bienes.iter().map(|b| b.coeficiente.valor()).sum(),
            poderes: a
                .bienes
                .iter()
                .filter(|b| b.poder_id.is_some())
                .map(|b| {
                    let estado = poderes
                        .iter()
                        .find(|p| Some(p.id) == b.poder_id)
                        .map_or("", |p| p.estado.as_str());
                    format!("{} ({estado})", b.codigo)
                })
                .collect(),
            entrada: propias
                .iter()
                .map(|p| p.entrada_en)
                .min()
                .map(|t| hora(t, tz))
                .unwrap_or_else(|| "no ingresó".into()),
            salidas: propias
                .iter()
                .filter_map(|p| {
                    p.salida_en.map(|s| {
                        format!(
                            "{} ({})",
                            hora(s, tz),
                            p.motivo_salida.map_or("", |m| m.as_str()).replace('_', " ")
                        )
                    })
                })
                .collect(),
        });
    }
    asistentes.sort_by(|a, b| a.nombre.cmp(&b.nombre));

    let mut preguntas = Vec::new();
    let mut por_unidades = false;
    for p in tx.preguntas(r.id).await? {
        if p.pregunta.estado == EstadoPregunta::Borrador {
            continue;
        }
        let escrutinio = match &p.resultado {
            Some(e) => Some(e.clone()),
            None if p.pregunta.estado == EstadoPregunta::Anulada => None,
            None => Some(escrutinio_de(tx, &p).await?),
        };
        por_unidades |= escrutinio.as_ref().is_some_and(|e| e.advertencia_unidades);
        let detalle = if p.pregunta.tipo_voto == TipoVoto::Nominal {
            tx.detalle_votos(p.pregunta.id)
                .await?
                .into_iter()
                .map(|d| VotoActa {
                    bien: d.codigo,
                    coeficiente: d.coeficiente.valor(),
                    opcion: p
                        .pregunta
                        .opcion(d.opcion_id)
                        .map(|o| o.texto.clone())
                        .unwrap_or_default(),
                    canal: d.canal.as_str().into(),
                    hora: hora(d.registrado_en, tz),
                    apoderado: d.apoderado,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut elegidos = Vec::new();
        for persona in elegidos_de(&p) {
            if let Some(x) = tx.persona(persona).await? {
                elegidos.push(x.nombre);
            }
        }
        preguntas.push(PreguntaActa {
            orden: p.orden,
            enunciado: p.pregunta.enunciado.clone(),
            tipo: p.pregunta.tipo.as_str().replace('_', " "),
            tipo_voto: p.pregunta.tipo_voto.as_str().into(),
            abierta: p.abierta_en.map(|t| hora(t, tz)),
            cerrada: p.cerrada_en.map(|t| hora(t, tz)),
            decision: if p.pregunta.estado == EstadoPregunta::Anulada {
                "anulada".into()
            } else {
                texto_decision(&p)
            },
            escrutinio,
            detalle,
            elegidos,
        });
    }

    let mut advertencias = Vec::new();
    if por_unidades {
        advertencias.push(
            "Algunas decisiones se tomaron por número de bienes privados (unidades). La Ley 675 de 2001 (arts. 45 y 46) exige coeficientes: riesgo de impugnación (art. 49)."
                .into(),
        );
    }
    if r.configuracion.efecto_mora != domain::EfectoMora::Informativo {
        advertencias.push(format!(
            "Efecto de la mora en esta asamblea: {} (decidido por la copropiedad).",
            r.configuracion.efecto_mora.as_str().replace('_', " ")
        ));
    }
    Ok(DatosActa {
        copropiedad: ficha.nombre,
        nit: ficha.nit.map(|n| n.to_string()),
        titulo: r.titulo.clone(),
        tipo: r.tipo.as_str().into(),
        convocatoria: r.convocatoria.as_str().replace('_', " "),
        modalidad: r.modalidad.as_str().into(),
        lugar: r.enlace_video.clone().or_else(|| r.lugar.clone()),
        fecha: hora(r.fecha_programada, tz),
        instalada: r.instalada_en.map(|t| hora(t, tz)),
        cerrada: r.cerrada_en.map(|t| hora(t, tz)),
        orden_del_dia: tx
            .puntos(r.id)
            .await?
            .into_iter()
            .map(|p| format!("{}. {}", p.orden, p.titulo))
            .collect(),
        quorum,
        asistentes,
        morosos: padron
            .iter()
            .filter(|b| b.moroso)
            .map(|b| codigo(b.bien_privado_id))
            .collect(),
        efecto_mora: r.configuracion.efecto_mora.as_str().replace('_', " "),
        preguntas,
        advertencias,
        generada: hora(ahora, tz),
    })
}

/// Genera (o regenera) el PDF del acta de una reunión cerrada y lo guarda como archivo con
/// datos personales. Se puede regenerar mientras no se apruebe.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si la reunión no está cerrada; [`AppError::Dominio`] si el acta
/// ya se aprobó.
pub async fn generar_acta<U, A, G>(
    unidad: &U,
    almacen: &A,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas + RepositorioArchivos,
    A: AlmacenArchivos,
    G: GeneradorActas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let datos = {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        let r = reunion_de(&mut tx, reunion).await?;
        if r.estado != EstadoReunion::Cerrada {
            return Err(AppError::Conflicto(
                "el acta se genera con la reunión cerrada".into(),
            ));
        }
        r.acta_estado.transicionar(EstadoActa::Generada)?;
        datos_acta(&mut tx, &r, reloj.ahora()).await?
    };
    let pdf = generador.generar(&datos).await?;
    let subido = ArchivoSubido {
        nombre_original: NombreArchivo::new(&format!("Acta {}.pdf", datos.titulo))
            .or_else(|_| NombreArchivo::new("Acta.pdf"))?,
        clasificacion: ClasificacionArchivo::Personal,
        contenido: pdf,
    };
    let tipo = TipoArchivo::validar(&subido.contenido)?;
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        let mut r = reunion_bloqueada(&mut tx, reunion).await?;
        r.acta_estado = r.acta_estado.transicionar(EstadoActa::Generada)?;
        registrar_en(&mut tx, &archivo).await?;
        r.acta_archivo_id = Some(archivo.id);
        r.acta_generada_en = Some(reloj.ahora());
        tx.actualizar_reunion(&r).await?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_ACTA, "reunion", reunion.as_uuid())
                .con_detalle(DetalleAuditoria::vacio().con("acta", "generada")?),
        )
        .await?;
        tx.confirmar().await?;
        Ok::<_, AppError>(r)
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Datos del acta en JSON (para revisarla en pantalla antes de generar el PDF).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`; [`AppError::NoEncontrado`].
pub async fn vista_previa_acta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<DatosActa>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    datos_acta(&mut tx, &r, reloj.ahora()).await
}

/// Paso del acta: subir el escaneo firmado, aprobarla (comisión verificadora) o publicarla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasoActa {
    /// Liga el PDF escaneado con las firmas de presidente y secretario (ya subido).
    Firmada(ArchivoId),
    Aprobar,
    Publicar,
}

/// Avanza el acta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si la transición no es válida; [`AppError::Conflicto`] si se aprueba
/// sin el escaneo firmado.
pub async fn avanzar_acta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    paso: PasoActa,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas + RepositorioArchivos,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, reunion).await?;
    let ahora = reloj.ahora();
    let nombre = match paso {
        PasoActa::Firmada(archivo) => {
            if r.acta_estado != EstadoActa::Generada {
                return Err(AppError::Conflicto("primero genere el acta".into()));
            }
            if tx.buscar(archivo).await?.is_none() {
                return Err(AppError::NoEncontrado("archivo"));
            }
            r.acta_firmada_archivo_id = Some(archivo);
            "firmada"
        }
        PasoActa::Aprobar => {
            if r.acta_firmada_archivo_id.is_none() {
                return Err(AppError::Conflicto(
                    "suba el acta firmada antes de aprobarla".into(),
                ));
            }
            r.acta_estado = r.acta_estado.transicionar(EstadoActa::Aprobada)?;
            r.acta_aprobada_en = Some(ahora);
            "aprobada"
        }
        PasoActa::Publicar => {
            r.acta_estado = r.acta_estado.transicionar(EstadoActa::Publicada)?;
            r.acta_publicada_en = Some(ahora);
            "publicada"
        }
    };
    tx.actualizar_reunion(&r).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ACTA, "reunion", reunion.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("acta", nombre)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Libro de actas: reuniones con el acta publicada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`.
pub async fn libro_actas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<Reunion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.actas_publicadas().await
}

/// PDF con rótulos genéricos numerados (`prefijo` + número) en Code 128 para imprimir.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.registrar`; [`AppError::Validacion`] si la
/// cantidad no está entre 1 y [`MAX_ROTULOS`] o los códigos no son válidos.
pub async fn hoja_rotulos<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    prefijo: &str,
    desde: u32,
    cantidad: u32,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
    G: GeneradorActas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    if !(1..=MAX_ROTULOS).contains(&cantidad) {
        return Err(AppError::Validacion(format!(
            "de 1 a {MAX_ROTULOS} rótulos por hoja"
        )));
    }
    let titulo = {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        tx.ficha().await?.map(|f| f.nombre).unwrap_or_default()
    };
    let digitos = (desde.saturating_add(cantidad)).to_string().len().max(4);
    let codigos = (desde..desde.saturating_add(cantidad))
        .map(|n| domain::CodigoRotulo::numerado(prefijo, n, digitos).map(|c| c.as_str().to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    generador.rotulos(&HojaRotulos { titulo, codigos }).await
}
