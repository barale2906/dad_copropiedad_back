//! Titulares (propietarios) de los bienes privados.
//!
//! - [`registrar_titulares`]: primer conjunto de titulares de un bien sin titulares vigentes.
//! - [`transferir_titularidad`]: venta o sucesión. Termina el conjunto vigente y crea el nuevo
//!   **en la misma transacción**: nunca quedan dos conjuntos solapados (invariante 5).
//! - [`designar_representante`]: quién representa al bien ante la copropiedad.
//!
//! Cada titular con cuenta en la plataforma recibe el rol derivado `propietario` (origen
//! `propiedad`, `origen_id` = la titularidad); al terminar la titularidad, el rol vence. Lo
//! asigna el sistema (no quien registra): por eso no aplica la regla de no escalamiento.

use domain::{
    Ambito, ArchivoId, BienPrivadoId, ConjuntoPermisos, ConjuntoTitulares, CuentaId,
    DireccionCorreo, Documento, OrigenAsignacion, Permiso, PersonaId, Telefono, TipoDocumento,
    TipoPersona, TitularNuevo, TitularidadId, copropiedad::texto_obligatorio,
};
use serde::{Deserialize, Serialize};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::roles::{SolicitudAsignacion, asignar_en},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioBienesPrivados, RepositorioPersonas, RepositorioRoles,
        RepositorioTitulares, Transaccion, UnidadDeTrabajo,
        copropiedades::{Persona, Titularidad},
    },
};

pub(crate) const ACCION_REGISTRAR: AccionAuditoria =
    AccionAuditoria::new("titulares.titularidad.crear");
pub(crate) const ACCION_TRANSFERIR: AccionAuditoria =
    AccionAuditoria::new("titulares.titularidad.transferir");
pub(crate) const ACCION_REPRESENTANTE: AccionAuditoria =
    AccionAuditoria::new("titulares.representante.designar");

/// Código del rol plantilla que reciben los titulares con cuenta.
pub const CODIGO_PROPIETARIO: &str = "propietario";

/// Persona nueva que se registra como titular.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatosPersona {
    pub tipo: TipoPersona,
    pub nombre: String,
    pub documento: Documento,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    /// Cuenta de la plataforma de esa persona (la encontró el administrador por usuario o
    /// correo exactos).
    pub cuenta_id: Option<CuentaId>,
    pub tipo_documento: Option<TipoDocumento>,
}

/// Quién es el titular: una persona ya registrada o una nueva. Si el documento de una nueva
/// ya existe en la copropiedad, se usa la persona existente (sus datos no se sobrescriben).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefPersona {
    Existente(PersonaId),
    Nueva(DatosPersona),
}

/// Titular solicitado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudTitular {
    pub persona: RefPersona,
    pub tipo: domain::TipoTitularidad,
    pub porcentaje: domain::Porcentaje,
    pub es_representante: bool,
}

/// Resuelve (o crea) la persona y vincula su cuenta si viene una.
pub(crate) async fn resolver_persona<T>(tx: &mut T, r: RefPersona) -> AppResult<(Persona, bool)>
where
    T: RepositorioPersonas,
{
    match r {
        RefPersona::Existente(id) => Ok((
            tx.persona(id)
                .await?
                .ok_or(AppError::NoEncontrado("persona"))?,
            false,
        )),
        RefPersona::Nueva(d) => {
            if let Some(mut existente) = tx.persona_por_documento(&d.documento).await? {
                if let Some(c) = d.cuenta_id {
                    match existente.cuenta_id {
                        None => {
                            tx.vincular_cuenta(existente.id, c).await?;
                            existente.cuenta_id = Some(c);
                        }
                        Some(actual) if actual != c => {
                            return Err(AppError::Conflicto(
                                "la persona ya está vinculada a otra cuenta".into(),
                            ));
                        }
                        Some(_) => {}
                    }
                }
                return Ok((existente, false));
            }
            let persona = Persona {
                telefono: d.telefono,
                correo: d.correo,
                cuenta_id: d.cuenta_id,
                tipo_documento: d.tipo_documento,
                ..Persona::basica(
                    PersonaId::nuevo(),
                    d.tipo,
                    texto_obligatorio("nombre", &d.nombre, 200)?,
                    Some(d.documento),
                )
            };
            tx.insertar_persona(&persona).await?;
            Ok((persona, true))
        }
    }
}

/// Inserta el conjunto de titulares y asigna el rol derivado a quienes tienen cuenta.
pub(crate) async fn insertar_conjunto<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    personas: &[Persona],
    conjunto: &ConjuntoTitulares,
    soporte: Option<ArchivoId>,
) -> AppResult<Vec<Titularidad>>
where
    T: RepositorioTitulares + RepositorioRoles + RegistroAuditoria,
{
    let ahora = reloj.ahora();
    let rol = tx
        .rol_por_codigo(CODIGO_PROPIETARIO)
        .await?
        .ok_or(AppError::NoEncontrado("rol propietario"))?;
    // El rol lo deriva el sistema del hecho registrado, no lo otorga quien registra.
    let ctx_sistema = Contexto {
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
        ..ctx.clone()
    };
    let mut creadas = Vec::new();
    for t in conjunto.titulares() {
        let titularidad = Titularidad {
            id: TitularidadId::nuevo(),
            bien_privado_id: bien,
            persona_id: t.persona_id,
            tipo: t.tipo,
            porcentaje: t.porcentaje,
            es_representante: t.es_representante,
            vigente_desde: ahora,
            vigente_hasta: None,
            soporte_archivo_id: soporte,
        };
        tx.insertar_titularidad(&titularidad).await?;
        if let Some(cuenta) = personas
            .iter()
            .find(|p| p.id == t.persona_id)
            .and_then(|p| p.cuenta_id)
        {
            asignar_en(
                tx,
                reloj,
                &ctx_sistema,
                SolicitudAsignacion {
                    cuenta_id: cuenta,
                    rol_id: rol.id,
                    vigente_desde: Some(ahora),
                    vigente_hasta: None,
                    origen: OrigenAsignacion::Propiedad,
                    origen_id: Some(titularidad.id.as_uuid()),
                },
            )
            .await?;
        }
        creadas.push(titularidad);
    }
    Ok(creadas)
}

async fn preparar<T>(
    tx: &mut T,
    solicitudes: Vec<SolicitudTitular>,
) -> AppResult<(Vec<Persona>, ConjuntoTitulares)>
where
    T: RepositorioPersonas,
{
    let mut personas = Vec::new();
    let mut titulares = Vec::new();
    for s in solicitudes {
        let (persona, _) = resolver_persona(tx, s.persona).await?;
        titulares.push(TitularNuevo {
            persona_id: persona.id,
            tipo: s.tipo,
            porcentaje: s.porcentaje,
            es_representante: s.es_representante,
        });
        personas.push(persona);
    }
    Ok((personas, ConjuntoTitulares::new(titulares)?))
}

async fn exigir_bien<T: RepositorioBienesPrivados>(
    tx: &mut T,
    bien: BienPrivadoId,
) -> AppResult<()> {
    tx.bien(bien)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))
        .map(|_| ())
}

/// Registra los titulares de un bien privado que aún no tiene.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `titulares.editar`; [`AppError::NoEncontrado`].
/// - [`AppError::Dominio`] si el conjunto no suma 100 % o no tiene un representante.
/// - [`AppError::Conflicto`] si ya tiene titulares vigentes (use una transferencia).
pub async fn registrar_titulares<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    solicitudes: Vec<SolicitudTitular>,
) -> AppResult<Vec<Titularidad>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioTitulares
        + RepositorioPersonas
        + RepositorioBienesPrivados
        + RepositorioRoles
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::TitularesEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_bien(&mut tx, bien).await?;
    if !tx.titulares_vigentes(bien).await?.is_empty() {
        return Err(AppError::Conflicto(
            "el bien privado ya tiene titulares; registre una transferencia".into(),
        ));
    }
    let (personas, conjunto) = preparar(&mut tx, solicitudes).await?;
    let creadas = insertar_conjunto(&mut tx, reloj, ctx, bien, &personas, &conjunto, None).await?;
    let detalle = DetalleAuditoria::vacio().con("titulares", creadas.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR, "bien_privado", bien.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(creadas)
}

/// Transfiere el bien privado: termina a los titulares vigentes (y vence sus roles derivados) y
/// registra a los nuevos, todo en una transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `titulares.editar`; [`AppError::NoEncontrado`].
/// - [`AppError::Dominio`] si el conjunto nuevo es inválido.
/// - [`AppError::Conflicto`] si el bien no tiene titulares vigentes (use el registro).
pub async fn transferir_titularidad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    solicitudes: Vec<SolicitudTitular>,
    soporte: Option<ArchivoId>,
) -> AppResult<Vec<Titularidad>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioTitulares
        + RepositorioPersonas
        + RepositorioBienesPrivados
        + RepositorioRoles
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::TitularesEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_bien(&mut tx, bien).await?;
    if tx.titulares_vigentes(bien).await?.is_empty() {
        return Err(AppError::Conflicto(
            "el bien privado no tiene titulares vigentes; regístrelos primero".into(),
        ));
    }
    let (personas, conjunto) = preparar(&mut tx, solicitudes).await?;
    let ahora = reloj.ahora();
    let terminadas = tx.terminar_titulares(bien, ahora).await?;
    let mut roles_vencidos = 0;
    for id in &terminadas {
        roles_vencidos += tx
            .terminar_por_origen(OrigenAsignacion::Propiedad, id.as_uuid(), ahora)
            .await?;
    }
    let creadas =
        insertar_conjunto(&mut tx, reloj, ctx, bien, &personas, &conjunto, soporte).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("titulares_anteriores", terminadas.len())?
        .con("titulares_nuevos", creadas.len())?
        .con("roles_vencidos", roles_vencidos)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TRANSFERIR, "bien_privado", bien.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(creadas)
}

/// Designa al representante del bien ante la copropiedad entre sus titulares vigentes.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `titulares.editar`; [`AppError::NoEncontrado`] si la persona no
/// es titular vigente del bien.
pub async fn designar_representante<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: BienPrivadoId,
    persona: PersonaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioTitulares + RegistroAuditoria,
{
    ctx.exigir(Permiso::TitularesEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx
        .titulares_vigentes(bien)
        .await?
        .iter()
        .any(|t| t.persona_id == persona)
    {
        return Err(AppError::NoEncontrado("titular vigente"));
    }
    tx.designar_representante(bien, persona).await?;
    let detalle = DetalleAuditoria::vacio().con("persona_id", persona.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REPRESENTANTE, "bien_privado", bien.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Titular con los datos de su persona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitularConPersona {
    pub titularidad: Titularidad,
    pub persona: Persona,
}

/// Historial de titulares del bien privado (vigentes y anteriores).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `titulares.ver`.
pub async fn listar_titulares<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<Vec<TitularConPersona>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioTitulares + RepositorioPersonas + RepositorioBienesPrivados,
{
    ctx.exigir(Permiso::TitularesVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_bien(&mut tx, bien).await?;
    let mut lista = Vec::new();
    for t in tx.historial_titulares(bien).await? {
        let persona = tx
            .persona(t.persona_id)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
        lista.push(TitularConPersona {
            titularidad: t,
            persona,
        });
    }
    Ok(lista)
}

#[cfg(test)]
mod tests {
    use domain::{Porcentaje, TenantId, TipoTitularidad};
    use rust_decimal::dec;

    use super::*;
    use crate::{
        casos_uso::bienes_privados::{crear_bien_privado, tests::nuevo_bien},
        puertos::{RelojSistema, unidad_trabajo::memoria::UnidadEnMemoria},
    };

    fn nueva(doc: &str, cuenta: Option<CuentaId>) -> RefPersona {
        RefPersona::Nueva(DatosPersona {
            tipo: TipoPersona::Natural,
            nombre: format!("Persona {doc}"),
            documento: Documento::new(doc).expect("válido"),
            telefono: None,
            correo: None,
            cuenta_id: cuenta,
            tipo_documento: None,
        })
    }

    fn titular(p: RefPersona, porcentaje: rust_decimal::Decimal, repr: bool) -> SolicitudTitular {
        SolicitudTitular {
            persona: p,
            tipo: TipoTitularidad::Propietario,
            porcentaje: Porcentaje::new(porcentaje).expect("válido"),
            es_representante: repr,
        }
    }

    async fn entorno() -> (UnidadEnMemoria, Contexto, BienPrivadoId) {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let bien = crear_bien_privado(&unidad, &ctx, nuevo_bien("101", None))
            .await
            .expect("crea")
            .id;
        (unidad, ctx, bien)
    }

    #[tokio::test]
    async fn registrar_da_el_rol_propietario_a_quien_tiene_cuenta() {
        let (unidad, ctx, bien) = entorno().await;
        let cuenta = CuentaId::nuevo();
        let creadas = registrar_titulares(
            &unidad,
            &RelojSistema,
            &ctx,
            bien,
            vec![
                titular(nueva("111", Some(cuenta)), dec!(50), true),
                titular(nueva("222", None), dec!(50), false),
            ],
        )
        .await
        .expect("registra");
        assert_eq!(creadas.len(), 2);
        let asignaciones = unidad.datos().asignaciones;
        assert_eq!(asignaciones.len(), 1);
        assert_eq!(asignaciones[0].1.cuenta_id, cuenta);
        assert_eq!(asignaciones[0].1.origen, OrigenAsignacion::Propiedad);

        // Un segundo registro sobre el mismo bien es una transferencia.
        assert!(matches!(
            registrar_titulares(
                &unidad,
                &RelojSistema,
                &ctx,
                bien,
                vec![titular(nueva("333", None), dec!(100), true)]
            )
            .await,
            Err(AppError::Conflicto(_))
        ));
    }

    #[tokio::test]
    async fn transferir_vence_el_rol_del_vendedor_y_no_solapa() {
        let (unidad, ctx, bien) = entorno().await;
        let vendedor = CuentaId::nuevo();
        let comprador = CuentaId::nuevo();
        registrar_titulares(
            &unidad,
            &RelojSistema,
            &ctx,
            bien,
            vec![titular(nueva("111", Some(vendedor)), dec!(100), false)],
        )
        .await
        .expect("registra");
        transferir_titularidad(
            &unidad,
            &RelojSistema,
            &ctx,
            bien,
            vec![titular(nueva("999", Some(comprador)), dec!(100), false)],
            None,
        )
        .await
        .expect("transfiere");

        let datos = unidad.datos();
        let ahora = RelojSistema.ahora();
        let vigente = |c: CuentaId| {
            datos.asignaciones.iter().any(|(_, a)| {
                a.cuenta_id == c && a.vigente_en(ahora + chrono::Duration::seconds(1))
            })
        };
        assert!(!vigente(vendedor), "el vendedor pierde el rol");
        assert!(vigente(comprador), "el comprador lo recibe");
        let abiertos: Vec<_> = datos
            .titularidades
            .iter()
            .filter(|t| t.vigente_hasta.is_none())
            .collect();
        assert_eq!(abiertos.len(), 1, "un solo conjunto vigente");
    }

    #[tokio::test]
    async fn conjunto_invalido_no_deja_nada() {
        let (unidad, ctx, bien) = entorno().await;
        assert!(matches!(
            registrar_titulares(
                &unidad,
                &RelojSistema,
                &ctx,
                bien,
                vec![
                    titular(nueva("111", None), dec!(60), true),
                    titular(nueva("222", None), dec!(30), false)
                ]
            )
            .await,
            Err(AppError::Dominio(domain::DominioError::TitularesInvalidos(
                _
            )))
        ));
        assert!(unidad.datos().personas.is_empty());
    }

    #[tokio::test]
    async fn representante_debe_ser_titular_vigente() {
        let (unidad, ctx, bien) = entorno().await;
        let creadas = registrar_titulares(
            &unidad,
            &RelojSistema,
            &ctx,
            bien,
            vec![
                titular(nueva("111", None), dec!(50), true),
                titular(nueva("222", None), dec!(50), false),
            ],
        )
        .await
        .expect("registra");
        designar_representante(&unidad, &ctx, bien, creadas[1].persona_id)
            .await
            .expect("designa");
        let lista = listar_titulares(&unidad, &ctx, bien).await.expect("lista");
        let repr: Vec<_> = lista
            .iter()
            .filter(|t| t.titularidad.es_representante)
            .map(|t| t.persona.id)
            .collect();
        assert_eq!(repr, vec![creadas[1].persona_id]);
        assert!(matches!(
            designar_representante(&unidad, &ctx, bien, PersonaId::nuevo()).await,
            Err(AppError::NoEncontrado(_))
        ));
    }
}
