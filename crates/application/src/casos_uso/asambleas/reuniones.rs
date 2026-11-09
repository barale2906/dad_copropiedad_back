//! Ciclo de vida de la reunión: crear y configurar, orden del día, documentos, convocar (padrón
//! y envío multicanal con constancia), abrir el registro, instalar con quórum, suspender,
//! reanudar, cerrar, cancelar y convocar la segunda reunión cuando no hubo quórum (art. 41).

use chrono::{DateTime, Months, TimeZone, Utc};
use domain::{
    ArchivoId, CalidadRepresentacion, Convocatoria, DominioError, EstadoPregunta, EstadoPunto,
    EstadoReunion, MetodoRegistro, Modalidad, MomentoQuorum, Permiso, PresenciaId, PuntoOrdenId,
    ReunionId, TipoDocumentoReunion, TipoReunion, asistente::CARGOS_PROHIBIBLES, reunion,
};

use super::{
    ACCION_CONVOCAR, ACCION_CREAR, ACCION_EDITAR, ACCION_PUNTO, ACCION_TRANSICIONAR, TxAsambleas,
    abrir,
    quorum::{calcular_quorum, tomar_foto},
    reunion_bloqueada, reunion_de, texto, texto_opcional,
    votaciones::{cerrar_en, resumen_final},
    zona_de,
};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{DetalleAuditoria, EventoAuditoria},
    casos_uso::avisos::IDIOMA_PLANTILLAS,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioAsambleas, Transaccion, UnidadDeTrabajo,
        asambleas::{
            AsistenteNuevo, CanalConvocatoria, ConfiguracionReunion, DocumentoReunion,
            EnvioConvocatoria, Presencia, PuntoOrden, Reunion,
        },
        correo::CorreoSaliente,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

/// Plantilla de utilidad «convocatoria_asamblea». Parámetros: copropiedad, título, fecha y
/// lugar o enlace. En consultas por escrito lleva el botón «Votar».
pub const PLANTILLA_CONVOCATORIA: &str = "convocatoria_asamblea";

/// Datos de una reunión nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaReunion {
    pub titulo: String,
    pub tipo: TipoReunion,
    pub convocatoria: Convocatoria,
    pub modalidad: Modalidad,
    pub fecha_programada: DateTime<Utc>,
    pub lugar: Option<String>,
    pub enlace_video: Option<String>,
    /// Obligatorio en consultas por escrito (a más tardar un mes después de la fecha).
    pub plazo_respuesta: Option<DateTime<Utc>>,
    pub configuracion: ConfiguracionReunion,
}

fn validar_configuracion(c: &ConfiguracionReunion) -> AppResult<()> {
    if let Some(cargo) = c
        .apoderados_prohibidos
        .iter()
        .find(|p| !CARGOS_PROHIBIBLES.contains(&p.as_str()))
    {
        return Err(AppError::Validacion(format!(
            "`{cargo}` no es un cargo que pueda prohibirse como apoderado"
        )));
    }
    if c.max_bienes_apoderado == Some(0) {
        return Err(AppError::Validacion(
            "`max_bienes_apoderado` debe ser mayor que 0".into(),
        ));
    }
    if c.temporizador_segundos
        .is_some_and(|s| !(10..=86_400).contains(&s))
    {
        return Err(AppError::Validacion(
            "el temporizador va de 10 segundos a 24 horas".into(),
        ));
    }
    Ok(())
}

fn validar_datos(n: &NuevaReunion) -> AppResult<()> {
    validar_configuracion(&n.configuracion)?;
    match (n.modalidad, n.plazo_respuesta) {
        (Modalidad::Escrita, Some(plazo)) => {
            let maximo = n
                .fecha_programada
                .checked_add_months(Months::new(reunion::MESES_CONSULTA_ESCRITA))
                .unwrap_or(n.fecha_programada);
            if plazo <= n.fecha_programada || plazo > maximo {
                return Err(AppError::Validacion(
                    "el plazo de la consulta por escrito va hasta un mes después de su inicio (art. 43)"
                        .into(),
                ));
            }
        }
        (Modalidad::Escrita, None) => {
            return Err(AppError::Validacion(
                "la consulta por escrito necesita `plazo_respuesta`".into(),
            ));
        }
        (_, Some(_)) => {
            return Err(AppError::Validacion(
                "`plazo_respuesta` solo aplica a la consulta por escrito".into(),
            ));
        }
        (_, None) => {}
    }
    Ok(())
}

fn detalle_configuracion(c: &ConfiguracionReunion) -> AppResult<DetalleAuditoria> {
    DetalleAuditoria::vacio()
        .con("base_computo", c.base_computo.as_str())?
        .con("efecto_mora", c.efecto_mora.as_str())?
        .con("max_bienes_apoderado", c.max_bienes_apoderado)?
        .con("exigir_soporte_poder", c.exigir_soporte_poder)?
        .con("apoderados_prohibidos", c.apoderados_prohibidos.clone())
}

/// Crea una reunión en borrador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.convocar`; [`AppError::Validacion`] si los datos o
/// la configuración son inválidos; [`AppError::ModuloNoHabilitado`].
pub async fn crear_reunion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaReunion,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    validar_datos(&nueva)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = Reunion {
        id: ReunionId::nuevo(),
        titulo: texto("titulo", &nueva.titulo, 200)?,
        tipo: nueva.tipo,
        convocatoria: nueva.convocatoria,
        modalidad: nueva.modalidad,
        estado: EstadoReunion::Borrador,
        fecha_programada: nueva.fecha_programada,
        lugar: texto_opcional("lugar", nueva.lugar.as_deref(), 300)?,
        enlace_video: texto_opcional("enlace_video", nueva.enlace_video.as_deref(), 500)?,
        plazo_respuesta: nueva.plazo_respuesta,
        reunion_origen_id: None,
        configuracion: nueva.configuracion,
        convocada_en: None,
        registro_abierto_en: None,
        instalada_en: None,
        cerrada_en: None,
        cancelada_en: None,
        motivo_cancelacion: None,
        acta_estado: domain::EstadoActa::Pendiente,
        acta_archivo_id: None,
        acta_firmada_archivo_id: None,
        acta_generada_en: None,
        acta_aprobada_en: None,
        acta_publicada_en: None,
        creada_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_reunion(&r).await?;
    let detalle = detalle_configuracion(&r.configuracion)?
        .con("tipo", r.tipo.as_str())?
        .con("modalidad", r.modalidad.as_str())?;
    tx.registrar(EventoAuditoria::ok(ACCION_CREAR, "reunion", r.id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Cambios a una reunión. Los datos de la convocatoria solo se editan en borrador; la
/// configuración (poderes, mora, base, temporizador) hasta antes de instalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CambiosReunion {
    pub datos: Option<NuevaReunion>,
    pub configuracion: Option<ConfiguracionReunion>,
}

/// Edita la reunión.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] con
/// `TransicionInvalida` si el estado ya no permite el cambio; [`AppError::Validacion`].
pub async fn editar_reunion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
    cambios: CambiosReunion,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, id).await?;
    if let Some(datos) = cambios.datos {
        exigir_editable(&r)?;
        validar_datos(&datos)?;
        r.titulo = texto("titulo", &datos.titulo, 200)?;
        r.tipo = datos.tipo;
        r.convocatoria = datos.convocatoria;
        r.modalidad = datos.modalidad;
        r.fecha_programada = datos.fecha_programada;
        r.lugar = texto_opcional("lugar", datos.lugar.as_deref(), 300)?;
        r.enlace_video = texto_opcional("enlace_video", datos.enlace_video.as_deref(), 500)?;
        r.plazo_respuesta = datos.plazo_respuesta;
        r.configuracion = datos.configuracion;
    }
    if let Some(c) = cambios.configuracion {
        if !matches!(
            r.estado,
            EstadoReunion::Borrador | EstadoReunion::Convocada | EstadoReunion::EnRegistro
        ) {
            return Err(AppError::Conflicto(
                "la configuración no cambia después de instalar la reunión".into(),
            ));
        }
        validar_configuracion(&c)?;
        r.configuracion = c;
    }
    tx.actualizar_reunion(&r).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR, "reunion", r.id.as_uuid())
            .con_detalle(detalle_configuracion(&r.configuracion)?),
    )
    .await?;
    tx.avisar_en_vivo(r.id).await?;
    tx.confirmar().await?;
    Ok(r)
}

fn exigir_editable(r: &Reunion) -> AppResult<()> {
    if r.estado.editable() {
        Ok(())
    } else {
        Err(AppError::Conflicto(
            "la reunión ya fue convocada: su convocatoria no se edita".into(),
        ))
    }
}

/// Una reunión con su orden del día y documentos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleReunion {
    pub reunion: Reunion,
    pub puntos: Vec<PuntoOrden>,
    pub documentos: Vec<DocumentoReunion>,
}

/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`; [`AppError::NoEncontrado`].
pub async fn ver_reunion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<DetalleReunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let reunion = reunion_de(&mut tx, id).await?;
    Ok(DetalleReunion {
        puntos: tx.puntos(id).await?,
        documentos: tx.documentos(id).await?,
        reunion,
    })
}

/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`.
pub async fn listar_reuniones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    estado: Option<EstadoReunion>,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Reunion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.reuniones(estado, pagina).await
}

// ---------------------------------------------------------------------------------------------
// Orden del día y documentos
// ---------------------------------------------------------------------------------------------

/// Datos de un punto del orden del día.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosPunto {
    pub orden: u16,
    pub titulo: String,
    pub descripcion: Option<String>,
}

/// Crea (`id = None`) o edita un punto del orden del día (solo en borrador).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si la
/// reunión ya se convocó; [`AppError::Validacion`].
pub async fn guardar_punto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: Option<PuntoOrdenId>,
    datos: DatosPunto,
) -> AppResult<PuntoOrden>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let DatosPunto {
        orden,
        titulo,
        descripcion,
    } = datos;
    if orden == 0 {
        return Err(AppError::Validacion("`orden` empieza en 1".into()));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    exigir_editable(&r)?;
    let puntos = tx.puntos(reunion).await?;
    if let Some(id) = id
        && !puntos.iter().any(|p| p.id == id)
    {
        return Err(AppError::NoEncontrado("punto del orden del día"));
    }
    if puntos.iter().any(|p| p.orden == orden && Some(p.id) != id) {
        return Err(AppError::Conflicto("ya hay un punto con ese orden".into()));
    }
    let punto = PuntoOrden {
        id: id.unwrap_or_else(PuntoOrdenId::nuevo),
        orden,
        titulo: texto("titulo", &titulo, 300)?,
        descripcion: texto_opcional("descripcion", descripcion.as_deref(), 4000)?,
        estado: EstadoPunto::Pendiente,
        iniciado_en: None,
        terminado_en: None,
    };
    tx.guardar_punto(reunion, &punto, id.is_none()).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PUNTO,
        "punto_orden_dia",
        punto.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(punto)
}

/// Elimina un punto (solo en borrador).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`].
pub async fn eliminar_punto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PuntoOrdenId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    exigir_editable(&reunion_de(&mut tx, reunion).await?)?;
    if !tx.puntos(reunion).await?.iter().any(|p| p.id == id) || !tx.eliminar_punto(id).await? {
        return Err(AppError::NoEncontrado("punto del orden del día"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_PUNTO,
        "punto_orden_dia",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Avanza un punto del orden del día durante la reunión (en curso, terminado, aplazado).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si la transición no es válida o la reunión no está instalada.
pub async fn cambiar_estado_punto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PuntoOrdenId,
    hacia: EstadoPunto,
) -> AppResult<PuntoOrden>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion)
        .await?
        .estado
        .exigir_instalada()?;
    let mut punto = tx
        .puntos(reunion)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(AppError::NoEncontrado("punto del orden del día"))?;
    punto.estado = punto.estado.transicionar(hacia)?;
    let ahora = reloj.ahora();
    match hacia {
        EstadoPunto::EnCurso => punto.iniciado_en = Some(ahora),
        EstadoPunto::Terminado | EstadoPunto::Aplazado => punto.terminado_en = Some(ahora),
        EstadoPunto::Pendiente => {}
    }
    tx.guardar_punto(reunion, &punto, false).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PUNTO, "punto_orden_dia", id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("estado", hacia.as_str())?),
    )
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    tx.confirmar().await?;
    Ok(punto)
}

/// Anexa un documento a la convocatoria (en borrador o ya convocada).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`].
pub async fn agregar_documento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    archivo_id: ArchivoId,
    tipo: TipoDocumentoReunion,
    titulo: &str,
) -> AppResult<DocumentoReunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    if !matches!(r.estado, EstadoReunion::Borrador | EstadoReunion::Convocada) {
        return Err(AppError::Conflicto(
            "los anexos se agregan antes del día de la reunión".into(),
        ));
    }
    let d = DocumentoReunion {
        archivo_id,
        tipo,
        titulo: texto("titulo", titulo, 200)?,
    };
    tx.agregar_documento(reunion, &d).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR,
        "reunion",
        reunion.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Quita un anexo.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn quitar_documento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    archivo_id: ArchivoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    if !tx.quitar_documento(reunion, archivo_id).await? {
        return Err(AppError::NoEncontrado("documento"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR,
        "reunion",
        reunion.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Transiciones
// ---------------------------------------------------------------------------------------------

/// Aplica la transición y sus marcas de tiempo.
pub(crate) fn transicionar(
    r: &mut Reunion,
    hacia: EstadoReunion,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    r.estado = r.estado.transicionar(hacia)?;
    match hacia {
        EstadoReunion::Convocada => r.convocada_en = Some(ahora),
        EstadoReunion::EnRegistro => r.registro_abierto_en = Some(ahora),
        EstadoReunion::Instalada => {
            r.instalada_en.get_or_insert(ahora);
        }
        EstadoReunion::Cerrada => r.cerrada_en = Some(ahora),
        EstadoReunion::Cancelada => r.cancelada_en = Some(ahora),
        EstadoReunion::Borrador | EstadoReunion::Suspendida => {}
    }
    Ok(())
}

async fn auditar_transicion<T: TxAsambleas>(tx: &mut T, r: &Reunion) -> AppResult<()> {
    tx.registrar(
        EventoAuditoria::ok(ACCION_TRANSICIONAR, "reunion", r.id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("estado", r.estado.as_str())?),
    )
    .await
}

/// Opciones del envío de la convocatoria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvioConvocatoriaOpciones {
    pub whatsapp: bool,
    pub correo: bool,
}

impl Default for EnvioConvocatoriaOpciones {
    fn default() -> Self {
        Self {
            whatsapp: true,
            correo: true,
        }
    }
}

/// Resumen de la convocatoria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoConvocatoria {
    pub reunion: Reunion,
    pub bienes_padron: u32,
    pub envios_whatsapp: u32,
    pub envios_correo: u32,
    /// Representantes sin teléfono ni correo (hay que convocarlos por otro medio).
    pub sin_contacto: u32,
}

/// Convoca la reunión: fija el padrón (coeficientes del cuadro vigente y representantes) y
/// envía la convocatoria por WhatsApp (plantilla) y correo a cada representante, dejando
/// constancia de cada envío. En una consulta por escrito, además, habilita a cada
/// representante y la deja instalada hasta su plazo.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está en
/// borrador; [`AppError::Validacion`] si no respeta la antelación de 15 días (ordinaria de
/// primera convocatoria, art. 39), no tiene orden del día o no hay coeficientes vigentes.
pub async fn convocar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
    envio: EnvioConvocatoriaOpciones,
) -> AppResult<ResultadoConvocatoria>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let resultado = convocar_en(&mut tx, reloj, ctx, id, envio).await?;
    tx.confirmar().await?;
    Ok(resultado)
}

pub(crate) async fn convocar_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
    envio: EnvioConvocatoriaOpciones,
) -> AppResult<ResultadoConvocatoria> {
    let ahora = reloj.ahora();
    let tz = zona_de(tx).await?;
    let mut r = reunion_bloqueada(tx, id).await?;
    let escrita = r.modalidad == Modalidad::Escrita;
    if r.convocatoria == Convocatoria::Primera
        && !escrita
        && !reunion::antelacion_suficiente(
            r.tipo,
            ahora.with_timezone(&tz).date_naive(),
            r.fecha_programada.with_timezone(&tz).date_naive(),
        )
    {
        return Err(AppError::Validacion(format!(
            "la reunión {} se convoca con al menos {} días calendario de antelación (art. 39)",
            r.tipo,
            reunion::ANTELACION_ORDINARIA_DIAS
        )));
    }
    if escrita {
        if tx.preguntas(id).await?.is_empty() {
            return Err(AppError::Validacion(
                "la consulta por escrito necesita al menos una pregunta".into(),
            ));
        }
    } else if tx.puntos(id).await?.is_empty() {
        return Err(AppError::Validacion(
            "la convocatoria debe incluir el orden del día (art. 39)".into(),
        ));
    }
    transicionar(&mut r, EstadoReunion::Convocada, ahora)?;

    let bienes = tx.bienes_para_padron().await?;
    if bienes.is_empty() {
        return Err(AppError::Validacion(
            "no hay bienes privados con coeficiente vigente: active el cuadro de coeficientes"
                .into(),
        ));
    }
    tx.insertar_padron(id, &bienes).await?;

    let ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    let mut representantes: Vec<_> = bienes.iter().filter_map(|b| b.representante).collect();
    representantes.sort();
    representantes.dedup();
    let sin_representante = bienes.iter().filter(|b| b.representante.is_none()).count();

    let mut resultado = ResultadoConvocatoria {
        reunion: r.clone(),
        bienes_padron: u32::try_from(bienes.len()).unwrap_or(u32::MAX),
        envios_whatsapp: 0,
        envios_correo: 0,
        sin_contacto: u32::try_from(sin_representante).unwrap_or(u32::MAX),
    };
    let fecha = r
        .fecha_programada
        .with_timezone(&tz)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let donde = r
        .enlace_video
        .clone()
        .or_else(|| r.lugar.clone())
        .unwrap_or_else(|| "por definir".into());
    for persona_id in representantes {
        let Some(persona) = tx.persona(persona_id).await? else {
            continue;
        };
        let mut asistente = None;
        if escrita {
            let a = AsistenteNuevo {
                id: domain::AsistenteId::nuevo(),
                reunion_id: id,
                persona_id,
                metodo: MetodoRegistro::Consulta,
                telefono: persona.telefono.as_ref().map(|t| t.as_str().to_owned()),
                registrado_por: ctx.actor_id,
                creado_en: ahora,
            };
            tx.insertar_asistente(&a).await?;
            for b in bienes
                .iter()
                .filter(|b| b.representante == Some(persona_id))
            {
                tx.vincular_bien(
                    id,
                    a.id,
                    b.bien_privado_id,
                    CalidadRepresentacion::Propietario,
                    None,
                    ahora,
                )
                .await?;
            }
            tx.abrir_presencia(
                id,
                &Presencia {
                    id: PresenciaId::nuevo(),
                    asistente_id: a.id,
                    entrada_en: ahora,
                    salida_en: None,
                    motivo_salida: None,
                    detalle_salida: None,
                    registrada_por: ctx.actor_id,
                },
            )
            .await?;
            asistente = Some(a.id);
        }
        let mut contactado = false;
        if envio.whatsapp
            && let Some(tel) = &persona.telefono
            && let Some(linea) = tx.elegir_linea().await?
        {
            let botones = asistente
                .map(|a| vec![super::whatsapp::payload_confirmar(ctx.tenant_id, a)])
                .unwrap_or_default();
            let evento = EventoSaliente {
                prioridad: PrioridadSalida::Recordatorio,
                mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                    linea: linea.clone(),
                    telefono: tel.as_str().to_owned(),
                    contenido: ContenidoSaliente::Plantilla {
                        nombre: PLANTILLA_CONVOCATORIA.into(),
                        idioma: IDIOMA_PLANTILLAS.into(),
                        parametros: vec![
                            super::corto(&ficha.nombre, 60),
                            super::corto(&r.titulo, 100),
                            fecha.clone(),
                            super::corto(&donde, 120),
                        ],
                        botones,
                    },
                }),
            };
            let outbox = tx.encolar_de_reunion(id, &evento).await?;
            tx.registrar_envio_convocatoria(id, persona_id, CanalConvocatoria::Whatsapp, outbox)
                .await?;
            if let Some(a) = asistente {
                tx.actualizar_medios(a, Some(tel.as_str()), Some(&linea), None, Some(outbox))
                    .await?;
            }
            resultado.envios_whatsapp += 1;
            contactado = true;
        }
        if envio.correo
            && let Some(correo) = &persona.correo
        {
            let evento = EventoSaliente {
                prioridad: PrioridadSalida::Recordatorio,
                mensaje: MensajeOutbox::Correo(CorreoSaliente {
                    para: correo.clone(),
                    asunto: format!("Convocatoria: {} — {}", r.titulo, ficha.nombre),
                    texto: texto_convocatoria(&ficha.nombre, &r, &fecha, &donde),
                    html: None,
                }),
            };
            let outbox = tx.encolar_de_reunion(id, &evento).await?;
            tx.registrar_envio_convocatoria(id, persona_id, CanalConvocatoria::Correo, outbox)
                .await?;
            resultado.envios_correo += 1;
            contactado = true;
        }
        if !contactado {
            resultado.sin_contacto += 1;
        }
    }

    if escrita {
        // La consulta queda «instalada» hasta su plazo: solo así sus preguntas reciben votos.
        transicionar(&mut r, EstadoReunion::EnRegistro, ahora)?;
        transicionar(&mut r, EstadoReunion::Instalada, ahora)?;
    }
    tx.actualizar_reunion(&r).await?;
    if escrita {
        for p in tx.preguntas(id).await? {
            if p.pregunta.estado == EstadoPregunta::Borrador {
                super::votaciones::abrir_en(tx, reloj, ctx, &r, p.pregunta.id, r.plazo_respuesta)
                    .await?;
            }
        }
    }
    let detalle = DetalleAuditoria::vacio()
        .con("bienes_padron", resultado.bienes_padron)?
        .con("envios_whatsapp", resultado.envios_whatsapp)?
        .con("envios_buzon", resultado.envios_correo)?
        .con("sin_contacto", resultado.sin_contacto)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONVOCAR, "reunion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    resultado.reunion = r;
    Ok(resultado)
}

fn texto_convocatoria(copropiedad: &str, r: &Reunion, fecha: &str, donde: &str) -> String {
    let mut t = format!(
        "{copropiedad}\n\nSe convoca a la {} «{}» ({} convocatoria, modalidad {}).\n\nFecha: {fecha}\nLugar o enlace: {donde}\n",
        if r.modalidad == Modalidad::Escrita {
            "consulta por escrito".to_owned()
        } else {
            format!("asamblea general {}", r.tipo)
        },
        r.titulo,
        r.convocatoria.as_str().replace('_', " "),
        r.modalidad
    );
    if let Some(plazo) = r.plazo_respuesta {
        t.push_str(&format!(
            "Plazo para responder: {}\n",
            plazo.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    t.push_str(
        "\nSi no puede asistir, puede otorgar poder a otra persona (Ley 675 de 2001, art. 37).\n",
    );
    t
}

/// Constancias de envío de la convocatoria.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.convocar`; [`AppError::NoEncontrado`].
pub async fn constancias_convocatoria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<Vec<EnvioConvocatoria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, id).await?;
    tx.envios_convocatoria(id).await
}

/// Abre el registro del día (la mesa empieza a registrar y el quórum corre en vivo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está convocada.
pub async fn abrir_registro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, id).await?;
    if r.modalidad == Modalidad::Escrita {
        return Err(AppError::Conflicto(
            "la consulta por escrito no tiene registro".into(),
        ));
    }
    transicionar(&mut r, EstadoReunion::EnRegistro, reloj.ahora())?;
    tx.actualizar_reunion(&r).await?;
    auditar_transicion(&mut tx, &r).await?;
    tx.avisar_en_vivo(id).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Instala la reunión si hay quórum (art. 45; en segunda convocatoria o por derecho propio,
/// cualquier número plural) y deja la foto del quórum para el acta.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está en
/// registro; [`AppError::Conflicto`] si no hay quórum.
pub async fn instalar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, id).await?;
    let ahora = reloj.ahora();
    transicionar(&mut r, EstadoReunion::Instalada, ahora)?;
    let q = calcular_quorum(&mut tx, &r).await?;
    if !q.hay_quorum(r.convocatoria, r.configuracion.base_computo) {
        return Err(AppError::Conflicto(format!(
            "no hay quórum: {}% de los coeficientes y {} de {} bienes privados; puede convocar la segunda reunión (art. 41)",
            (q.coeficientes.fraccion() * rust_decimal::Decimal::ONE_HUNDRED).round_dp(2),
            q.unidades.representados,
            q.unidades.total
        )));
    }
    tomar_foto(&mut tx, &r, None, MomentoQuorum::Instalacion, q, ahora).await?;
    tx.actualizar_reunion(&r).await?;
    auditar_transicion(&mut tx, &r).await?;
    tx.avisar_en_vivo(id).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Suspende (receso) o reanuda la reunión.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la
/// transición no es válida; [`AppError::Conflicto`] si hay preguntas abiertas al suspender.
pub async fn suspender_o_reanudar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
    suspender: bool,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, id).await?;
    let hacia = if suspender {
        if tx
            .preguntas(id)
            .await?
            .iter()
            .any(|p| p.pregunta.estado == EstadoPregunta::Abierta)
        {
            return Err(AppError::Conflicto(
                "cierre las preguntas abiertas antes del receso".into(),
            ));
        }
        EstadoReunion::Suspendida
    } else {
        EstadoReunion::Instalada
    };
    transicionar(&mut r, hacia, reloj.ahora())?;
    tx.actualizar_reunion(&r).await?;
    auditar_transicion(&mut tx, &r).await?;
    tx.avisar_en_vivo(id).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Cierra la reunión: cierra las preguntas abiertas (con su escrutinio) y envía a cada
/// asistente con WhatsApp un único resumen de los resultados.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la
/// transición no es válida.
pub async fn cerrar_reunion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = cerrar_reunion_en(&mut tx, reloj, ctx, id).await?;
    tx.confirmar().await?;
    Ok(r)
}

pub(crate) async fn cerrar_reunion_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
) -> AppResult<Reunion> {
    let mut r = reunion_bloqueada(tx, id).await?;
    let ahora = reloj.ahora();
    transicionar(&mut r, EstadoReunion::Cerrada, ahora)?;
    for p in tx.preguntas(id).await? {
        if p.pregunta.estado == EstadoPregunta::Abierta {
            cerrar_en(tx, reloj, ctx, &r, p.pregunta.id).await?;
        }
    }
    tx.actualizar_reunion(&r).await?;
    resumen_final(tx, &r).await?;
    auditar_transicion(tx, &r).await?;
    tx.avisar_en_vivo(id).await?;
    Ok(r)
}

/// Cancela la reunión (antes de instalarla).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.convocar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si ya se instaló.
pub async fn cancelar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReunionId,
    motivo: &str,
) -> AppResult<Reunion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut r = reunion_bloqueada(&mut tx, id).await?;
    transicionar(&mut r, EstadoReunion::Cancelada, reloj.ahora())?;
    r.motivo_cancelacion = Some(texto("motivo", motivo, 500)?);
    tx.actualizar_reunion(&r).await?;
    auditar_transicion(&mut tx, &r).await?;
    tx.avisar_en_vivo(id).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Sin quórum en primera convocatoria: cancela la reunión y convoca la de segunda para el
/// tercer día hábil siguiente a las 8:00 p. m. (art. 41), con el mismo orden del día y las
/// mismas preguntas, y envía la convocatoria.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si la
/// reunión no es de primera convocatoria, no está en registro o sí tiene quórum.
pub async fn convocar_segunda<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    origen: ReunionId,
    envio: EnvioConvocatoriaOpciones,
) -> AppResult<ResultadoConvocatoria>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let tz = zona_de(&mut tx).await?;
    let mut primera = reunion_bloqueada(&mut tx, origen).await?;
    if primera.convocatoria != Convocatoria::Primera || primera.estado != EstadoReunion::EnRegistro
    {
        return Err(AppError::Conflicto(
            "la segunda convocatoria sale de una reunión de primera convocatoria en registro"
                .into(),
        ));
    }
    let q = calcular_quorum(&mut tx, &primera).await?;
    if q.hay_quorum(Convocatoria::Primera, primera.configuracion.base_computo) {
        return Err(AppError::Conflicto(
            "la reunión tiene quórum: instálela".into(),
        ));
    }
    let ahora = reloj.ahora();
    let local = reunion::fecha_segunda_convocatoria(
        primera.fecha_programada.with_timezone(&tz).date_naive(),
    )
    .ok_or(DominioError::DatoInvalido {
        campo: "fecha_programada",
        regla: "fuera de rango",
    })?;
    let fecha = tz
        .from_local_datetime(&local)
        .earliest()
        .map(|f| f.with_timezone(&Utc))
        .ok_or(DominioError::DatoInvalido {
            campo: "fecha_programada",
            regla: "hora local inexistente",
        })?;
    transicionar(&mut primera, EstadoReunion::Cancelada, ahora)?;
    primera.motivo_cancelacion = Some("sin quórum: se convoca segunda reunión (art. 41)".into());
    tx.actualizar_reunion(&primera).await?;
    auditar_transicion(&mut tx, &primera).await?;

    let segunda = Reunion {
        id: ReunionId::nuevo(),
        convocatoria: Convocatoria::Segunda,
        estado: EstadoReunion::Borrador,
        fecha_programada: fecha,
        reunion_origen_id: Some(origen),
        convocada_en: None,
        registro_abierto_en: None,
        instalada_en: None,
        cerrada_en: None,
        cancelada_en: None,
        motivo_cancelacion: None,
        creada_por: ctx.actor_id,
        creado_en: ahora,
        ..primera.clone()
    };
    tx.insertar_reunion(&segunda).await?;
    for p in tx.puntos(origen).await? {
        let nuevo = PuntoOrden {
            id: PuntoOrdenId::nuevo(),
            estado: EstadoPunto::Pendiente,
            iniciado_en: None,
            terminado_en: None,
            ..p
        };
        tx.guardar_punto(segunda.id, &nuevo, true).await?;
    }
    for d in tx.documentos(origen).await? {
        tx.agregar_documento(segunda.id, &d).await?;
    }
    super::votaciones::copiar_preguntas(&mut tx, origen, segunda.id).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CREAR,
        "reunion",
        segunda.id.as_uuid(),
    ))
    .await?;
    let resultado = convocar_en(&mut tx, reloj, ctx, segunda.id, envio).await?;
    tx.avisar_en_vivo(origen).await?;
    tx.confirmar().await?;
    Ok(resultado)
}
