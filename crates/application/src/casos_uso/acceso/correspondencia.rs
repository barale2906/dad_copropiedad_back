//! Correspondencia: la portería registra paquetes, sobres y recibos; se avisa al primer
//! respondiente y, cada 24 h hasta la entrega, a todos los residentes (fuera del horario de
//! silencio y hasta el tope de días; luego se alerta a la administración). La entrega exige
//! firma o foto de quien recoge, elegido del listado del bien o anotado como tercero.

use chrono::{DateTime, TimeDelta, Utc};
use domain::{
    BienPrivadoId, ClasificacionArchivo, CorrespondenciaId, Documento, EstadoCorrespondencia,
    NivelAviso, NombreArchivo, Permiso, PersonaId, QuienRecoge, SoporteEntrega, TipoArchivo,
    TipoCorrespondencia,
    acceso::{destinatarios_aviso, fuera_de_silencio, proximo_aviso_correspondencia},
    copropiedad::{texto_obligatorio, texto_opcional},
};

use super::{
    AvisoAcceso, PlantillaAviso, TxAcceso, abrir, avisar_bien, codigo_bien, hoy, tiene, vinculados,
    zona_ph,
};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    outbox::PrioridadSalida,
    puertos::{
        AlmacenArchivos, Reloj, RepositorioAcceso, Transaccion, UnidadDeTrabajo,
        acceso::{Correspondencia, EntregaCorrespondencia, FiltroCorrespondencia},
        archivos::Archivo,
    },
};

pub(crate) const ACCION_RECIBIR: AccionAuditoria =
    AccionAuditoria::new("acceso.correspondencia.recibir");
pub(crate) const ACCION_ENTREGAR: AccionAuditoria =
    AccionAuditoria::new("acceso.correspondencia.entregar");
pub(crate) const ACCION_DEVOLVER: AccionAuditoria =
    AccionAuditoria::new("acceso.correspondencia.devolver");

/// Plantilla «correspondencia_recibida». Parámetros: tipo, bien privado, días en portería.
pub const PLANTILLA_CORRESPONDENCIA_RECIBIDA: &str = "correspondencia_recibida";
/// Plantilla «correspondencia_entregada». Parámetros: tipo, bien privado, quién recogió.
pub const PLANTILLA_CORRESPONDENCIA_ENTREGADA: &str = "correspondencia_entregada";

/// Datos que registra la portería.
#[derive(Debug, Clone)]
pub struct NuevaCorrespondencia {
    pub bien_privado_id: BienPrivadoId,
    pub tipo: TipoCorrespondencia,
    pub remitente: Option<String>,
    pub guia: Option<String>,
    pub ubicacion: Option<String>,
    /// Foto del paquete (opcional).
    pub foto: Option<Vec<u8>>,
}

/// Quién recoge, como llega de la portería.
#[derive(Debug, Clone)]
pub enum QuienRecogeEntrada {
    Registrada(PersonaId),
    Tercero { nombre: String, documento: String },
}

/// Entrega con sus soportes (bytes de la firma PNG y/o de la foto).
#[derive(Debug, Clone)]
pub struct NuevaEntrega {
    pub quien: QuienRecogeEntrada,
    pub firma: Option<Vec<u8>>,
    pub foto: Option<Vec<u8>>,
}

/// Persona que puede recoger (listado de la portería al entregar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaParaEntrega {
    pub persona_id: PersonaId,
    pub nombre: String,
    /// `residente` o `propietario`.
    pub vinculo: &'static str,
}

/// Correspondencia con su entrega.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleCorrespondencia {
    pub correspondencia: Correspondencia,
    pub entrega: Option<EntregaCorrespondencia>,
}

fn texto_tipo(t: TipoCorrespondencia) -> &'static str {
    match t {
        TipoCorrespondencia::Paquete => "un paquete",
        TipoCorrespondencia::Sobre => "un sobre",
        TipoCorrespondencia::Recibo => "un recibo",
        TipoCorrespondencia::Otro => "correspondencia",
    }
}

/// Guarda los bytes de un soporte en el almacén (antes de la transacción).
async fn guardar_soporte(
    almacen: &impl AlmacenArchivos,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: &str,
    contenido: Vec<u8>,
) -> AppResult<Archivo> {
    let tipo = TipoArchivo::validar(&contenido)?;
    let subido = ArchivoSubido {
        nombre_original: NombreArchivo::new(nombre)?,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    Ok(archivo)
}

async fn limpiar(almacen: &impl AlmacenArchivos, ctx: &Contexto, archivos: &[Archivo]) {
    for a in archivos {
        borrar_huerfano(almacen, ctx, a).await;
    }
}

async fn auditar<T: TxAcceso>(
    tx: &mut T,
    accion: AccionAuditoria,
    c: &Correspondencia,
) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("estado", c.estado.as_str())?
        .con("tipo", c.tipo.as_str())?
        .con("bien_privado_id", c.bien_privado_id.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(accion, "correspondencia", c.id.as_uuid()).con_detalle(detalle),
    )
    .await
}

/// Aviso de correspondencia pendiente.
pub(crate) async fn aviso_recibida<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    c: &Correspondencia,
    ahora: DateTime<Utc>,
) -> AppResult<AvisoAcceso> {
    let codigo = codigo_bien(tx, c.bien_privado_id).await?;
    let dias = (ahora - c.recibida_en).num_days().max(0);
    let mut datos = std::collections::BTreeMap::new();
    datos.insert("tipo".into(), "correspondencia".into());
    datos.insert("copropiedad_id".into(), ctx.tenant_id.to_string());
    datos.insert("correspondencia_id".into(), c.id.to_string());
    let cuerpo = if dias == 0 {
        format!(
            "Llegó {} para {codigo}. Recójalo en portería.",
            texto_tipo(c.tipo)
        )
    } else {
        format!(
            "Tiene {} en portería para {codigo} hace {dias} día(s).",
            texto_tipo(c.tipo)
        )
    };
    Ok(AvisoAcceso {
        prioridad: PrioridadSalida::Evento,
        titulo: "Correspondencia en portería".into(),
        cuerpo,
        datos,
        plantilla: PlantillaAviso {
            nombre: PLANTILLA_CORRESPONDENCIA_RECIBIDA,
            parametros: vec![texto_tipo(c.tipo).into(), codigo, dias.to_string()],
            botones: Vec::new(),
        },
    })
}

/// Envía el aviso que corresponde (nivel según los avisos previos) y programa el siguiente.
pub(crate) async fn avisar_correspondencia<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    c: &mut Correspondencia,
    ahora: DateTime<Utc>,
) -> AppResult<u32> {
    let tz = zona_ph(tx).await?;
    let config = tx.configuracion_acceso().await?;
    let aviso = aviso_recibida(tx, ctx, c, ahora).await?;
    let nivel = NivelAviso::segun_avisos_previos(c.avisos_enviados);
    let n = avisar_bien(tx, c.bien_privado_id, nivel, hoy(ahora, tz), &aviso).await?;
    c.avisos_enviados += 1;
    c.ultimo_aviso_en = Some(ahora);
    c.siguiente_aviso_en = proximo_aviso_correspondencia(c.recibida_en, ahora, &config, tz);
    Ok(n)
}

/// Registra la correspondencia y avisa al primer respondiente (o programa el aviso para el fin
/// del horario de silencio).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] con datos o foto inválidos.
pub async fn registrar_correspondencia<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaCorrespondencia,
) -> AppResult<Correspondencia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let remitente = texto_opcional("remitente", n.remitente.as_deref(), 120)?;
    let guia = texto_opcional("guia", n.guia.as_deref(), 60)?;
    let ubicacion = texto_opcional("ubicacion", n.ubicacion.as_deref(), 60)?;
    let ahora = reloj.ahora();
    let foto = match n.foto {
        Some(b) => Some(guardar_soporte(almacen, reloj, ctx, "paquete", b).await?),
        None => None,
    };
    let resultado = async {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        codigo_bien(&mut tx, n.bien_privado_id).await?;
        if let Some(a) = &foto {
            registrar_en(&mut tx, a).await?;
        }
        let tz = zona_ph(&mut tx).await?;
        let config = tx.configuracion_acceso().await?;
        let mut c = Correspondencia {
            id: CorrespondenciaId::nuevo(),
            bien_privado_id: n.bien_privado_id,
            tipo: n.tipo,
            estado: EstadoCorrespondencia::Recibida,
            remitente,
            guia,
            ubicacion,
            foto_archivo_id: foto.as_ref().map(|a| a.id),
            recibida_por: ctx.actor_id,
            recibida_en: ahora,
            avisos_enviados: 0,
            ultimo_aviso_en: None,
            siguiente_aviso_en: Some(fuera_de_silencio(ahora, tz, config.silencio)),
            alerta_administracion_en: None,
            cerrada_en: None,
            motivo_devolucion: None,
        };
        tx.insertar_correspondencia(&c).await?;
        if c.siguiente_aviso_en.is_some_and(|s| s <= ahora) {
            avisar_correspondencia(&mut tx, ctx, &mut c, ahora).await?;
            tx.actualizar_correspondencia(&c).await?;
        }
        auditar(&mut tx, ACCION_RECIBIR, &c).await?;
        tx.confirmar().await?;
        Ok(c)
    }
    .await;
    if resultado.is_err() {
        limpiar(almacen, ctx, foto.as_slice()).await;
    }
    resultado
}

/// Personas que pueden recoger: residentes adultos y propietarios del bien.
pub(crate) async fn personas_para_entrega_en<T: TxAcceso>(
    tx: &mut T,
    bien: BienPrivadoId,
    ahora: DateTime<Utc>,
) -> AppResult<Vec<PersonaParaEntrega>> {
    let tz = zona_ph(tx).await?;
    let (residentes, titulares) = vinculados(tx, bien).await?;
    let adultos = destinatarios_aviso(NivelAviso::Todos, &residentes, &[], hoy(ahora, tz));
    let mut v = Vec::new();
    for p in adultos {
        if let Some(x) = tx.persona(p).await? {
            v.push(PersonaParaEntrega {
                persona_id: p,
                nombre: x.nombre,
                vinculo: "residente",
            });
        }
    }
    for t in titulares {
        if v.iter().any(|x| x.persona_id == t.persona) {
            continue;
        }
        if let Some(x) = tx.persona(t.persona).await? {
            v.push(PersonaParaEntrega {
                persona_id: t.persona,
                nombre: x.nombre,
                vinculo: "propietario",
            });
        }
    }
    Ok(v)
}

/// Listado de quienes pueden recoger correspondencia del bien.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.correspondencia.entregar`.
pub async fn personas_para_entrega<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<Vec<PersonaParaEntrega>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCorrespondenciaEntregar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    personas_para_entrega_en(&mut tx, bien, reloj.ahora()).await
}

/// Entrega la correspondencia: quien recoge (del listado o tercero) y firma o foto. Se guarda
/// el soporte por el plazo de la PH y se confirma al primer respondiente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.correspondencia.entregar`; [`AppError::Dominio`] sin
/// firma ni foto o con un documento inválido; [`AppError::Validacion`] si la persona no es del
/// bien; [`AppError::Conflicto`] si ya se entregó.
pub async fn entregar_correspondencia<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CorrespondenciaId,
    e: NuevaEntrega,
) -> AppResult<DetalleCorrespondencia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::AccesoCorrespondenciaEntregar)?;
    if e.firma.is_none() && e.foto.is_none() {
        SoporteEntrega::new(None, None)?;
    }
    let quien = match e.quien {
        QuienRecogeEntrada::Registrada(p) => QuienRecoge::Registrada(p),
        QuienRecogeEntrada::Tercero { nombre, documento } => QuienRecoge::Tercero {
            nombre: texto_obligatorio("nombre", &nombre, 120)?,
            documento: Documento::new(&documento)?,
        },
    };
    let ahora = reloj.ahora();
    let mut guardados = Vec::new();
    if let Some(b) = e.firma {
        guardados.push((
            true,
            guardar_soporte(almacen, reloj, ctx, "firma", b).await?,
        ));
    }
    if let Some(b) = e.foto {
        match guardar_soporte(almacen, reloj, ctx, "foto-entrega", b).await {
            Ok(a) => guardados.push((false, a)),
            Err(err) => {
                limpiar(
                    almacen,
                    ctx,
                    &guardados.iter().map(|(_, a)| a.clone()).collect::<Vec<_>>(),
                )
                .await;
                return Err(err);
            }
        }
    }
    let archivos: Vec<Archivo> = guardados.iter().map(|(_, a)| a.clone()).collect();
    let firma = guardados.iter().find(|(f, _)| *f).map(|(_, a)| a.id);
    let foto = guardados.iter().find(|(f, _)| !*f).map(|(_, a)| a.id);
    let resultado = async {
        let soporte = SoporteEntrega::new(firma, foto)?;
        let mut tx = abrir(unidad, reloj, ctx).await?;
        let mut c = tx
            .correspondencia_para_actualizar(id)
            .await?
            .ok_or(AppError::NoEncontrado("correspondencia"))?;
        c.estado = c.estado.transicionar(EstadoCorrespondencia::Entregada)?;
        let (persona_id, tercero_nombre, tercero_documento, nombre_aviso) = match &quien {
            QuienRecoge::Registrada(p) => {
                let personas = personas_para_entrega_en(&mut tx, c.bien_privado_id, ahora).await?;
                let x = personas.into_iter().find(|x| x.persona_id == *p).ok_or(
                    AppError::Validacion(
                        "la persona no está registrada en ese bien privado".into(),
                    ),
                )?;
                (Some(*p), None, None, x.nombre)
            }
            QuienRecoge::Tercero { nombre, documento } => (
                None,
                Some(nombre.clone()),
                Some(documento.clone()),
                "un tercero autorizado".to_owned(),
            ),
        };
        for a in &archivos {
            registrar_en(&mut tx, a).await?;
        }
        let config = tx.configuracion_acceso().await?;
        let entrega = EntregaCorrespondencia {
            correspondencia_id: c.id,
            persona_id,
            tercero_nombre,
            tercero_documento,
            firma_archivo_id: soporte.firma(),
            foto_archivo_id: soporte.foto(),
            entregada_por: ctx.actor_id,
            entregada_en: ahora,
            soportes_hasta: ahora + TimeDelta::days(i64::from(config.retencion_soportes_dias)),
        };
        tx.insertar_entrega(&entrega).await?;
        c.cerrada_en = Some(ahora);
        c.siguiente_aviso_en = None;
        tx.actualizar_correspondencia(&c).await?;
        // Confirmación al primer respondiente (no al que recogió necesariamente).
        let codigo = codigo_bien(&mut tx, c.bien_privado_id).await?;
        let tz = zona_ph(&mut tx).await?;
        let mut datos = std::collections::BTreeMap::new();
        datos.insert("tipo".into(), "correspondencia_entregada".into());
        datos.insert("copropiedad_id".into(), ctx.tenant_id.to_string());
        datos.insert("correspondencia_id".into(), c.id.to_string());
        let aviso = AvisoAcceso {
            prioridad: PrioridadSalida::Evento,
            titulo: "Correspondencia entregada".into(),
            cuerpo: format!(
                "Se entregó {} de {codigo} a {nombre_aviso}.",
                texto_tipo(c.tipo)
            ),
            datos,
            plantilla: PlantillaAviso {
                nombre: PLANTILLA_CORRESPONDENCIA_ENTREGADA,
                parametros: vec![texto_tipo(c.tipo).into(), codigo, nombre_aviso],
                botones: Vec::new(),
            },
        };
        avisar_bien(
            &mut tx,
            c.bien_privado_id,
            NivelAviso::PrimerRespondiente,
            hoy(ahora, tz),
            &aviso,
        )
        .await?;
        auditar(&mut tx, ACCION_ENTREGAR, &c).await?;
        tx.confirmar().await?;
        Ok(DetalleCorrespondencia {
            correspondencia: c,
            entrega: Some(entrega),
        })
    }
    .await;
    if resultado.is_err() {
        limpiar(almacen, ctx, &archivos).await;
    }
    resultado
}

/// Devuelve la correspondencia al remitente (deja de avisarse).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.correspondencia.entregar`; [`AppError::Dominio`] si ya
/// se cerró o el motivo es inválido.
pub async fn devolver_correspondencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CorrespondenciaId,
    motivo: &str,
) -> AppResult<Correspondencia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCorrespondenciaEntregar)?;
    let motivo = texto_obligatorio("motivo", motivo, 300)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut c = tx
        .correspondencia_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("correspondencia"))?;
    c.estado = c.estado.transicionar(EstadoCorrespondencia::Devuelta)?;
    c.cerrada_en = Some(ahora);
    c.siguiente_aviso_en = None;
    c.motivo_devolucion = Some(motivo);
    tx.actualizar_correspondencia(&c).await?;
    auditar(&mut tx, ACCION_DEVOLVER, &c).await?;
    tx.confirmar().await?;
    Ok(c)
}

async fn exigir_ver_bien<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
) -> AppResult<()> {
    if [
        Permiso::AccesoRegistrar,
        Permiso::AccesoBitacoraVer,
        Permiso::AccesoCorrespondenciaEntregar,
    ]
    .into_iter()
    .any(|p| tiene(ctx, p))
    {
        return Ok(());
    }
    let (Some(b), Some(cuenta)) = (bien, ctx.actor_id) else {
        return Err(AppError::SinPermiso(Permiso::AccesoBitacoraVer));
    };
    if tx.cuenta_vinculada_a_bien(cuenta, b).await? {
        Ok(())
    } else {
        Err(AppError::SinPermiso(Permiso::AccesoBitacoraVer))
    }
}

/// Correspondencia (más reciente primero). Un residente o propietario ve la de su bien.
///
/// # Errors
///
/// [`AppError::SinPermiso`].
pub async fn listar_correspondencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroCorrespondencia,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Correspondencia>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    exigir_ver_bien(&mut tx, ctx, filtro.bien_privado_id).await?;
    tx.correspondencias(&filtro, pagina).await
}

/// Correspondencia con su entrega. Los datos del tercero que recogió solo los ve la portería.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn ver_correspondencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CorrespondenciaId,
) -> AppResult<DetalleCorrespondencia>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let c = tx
        .correspondencia(id)
        .await?
        .ok_or(AppError::NoEncontrado("correspondencia"))?;
    exigir_ver_bien(&mut tx, ctx, Some(c.bien_privado_id)).await?;
    let mut entrega = tx.entrega(id).await?;
    if !tiene(ctx, Permiso::AccesoCorrespondenciaEntregar)
        && let Some(e) = entrega.as_mut()
    {
        e.tercero_documento = None;
    }
    Ok(DetalleCorrespondencia {
        correspondencia: c,
        entrega,
    })
}
