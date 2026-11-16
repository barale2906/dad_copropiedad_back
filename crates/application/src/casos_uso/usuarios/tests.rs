use chrono::{DateTime, TimeZone, Utc};
use domain::{Ambito, ConjuntoPermisos, TenantId};

use super::*;
use crate::puertos::unidad_trabajo::memoria::{
    UnidadEnMemoria,
    usuarios::{CuentaMemoria, DatosUsuarios},
};

struct RelojFijo;

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 16, 9, 0, 0)
            .single()
            .expect("fecha válida")
    }
}

fn admin(tenant: TenantId) -> Contexto {
    Contexto::de_cuenta(
        tenant,
        CuentaId::nuevo(),
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    )
}

async fn rol(unidad: &UnidadEnMemoria, ctx: &Contexto, codigo: &str) -> RolId {
    unidad
        .iniciar(ctx)
        .await
        .expect("inicia")
        .rol_por_codigo(codigo)
        .await
        .expect("consulta")
        .expect("plantilla")
        .id
}

fn correo(c: &str) -> DireccionCorreo {
    DireccionCorreo::new(c).expect("correo válido")
}

fn nuevo(c: &str, rol_id: RolId) -> NuevoUsuario {
    NuevoUsuario {
        nombre: "Ana Pérez".into(),
        correo: correo(c),
        usuario: None,
        telefono: None,
        documento: None,
        rol_id,
    }
}

fn asuntos(unidad: &UnidadEnMemoria) -> Vec<String> {
    unidad
        .outbox()
        .into_iter()
        .filter_map(|(_, e)| match e.mensaje {
            MensajeOutbox::Correo(c) => Some(c.asunto),
            _ => None,
        })
        .collect()
}

fn con_cuenta(unidad: &UnidadEnMemoria, cuenta: CuentaMemoria) {
    unidad.con_datos(|d| d.usuarios.cuentas.push(cuenta));
}

#[tokio::test]
async fn crea_la_cuenta_pendiente_con_su_rol_persona_y_codigo() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let contador = rol(&unidad, &ctx, "contador").await;

    let creado = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador))
        .await
        .expect("crea");

    assert!(creado.cuenta_nueva);
    assert!(creado.asignacion.is_some());
    let datos = unidad.datos();
    let DatosUsuarios { cuentas, codigos } = &datos.usuarios;
    assert!(cuentas[0].pendiente);
    assert_eq!(codigos[0].cuenta, creado.cuenta_id);
    assert_eq!(
        codigos[0].expira_en,
        RelojFijo.ahora() + VIDA_CODIGO_DEFINIR
    );
    let persona = datos.personas.iter().find(|p| p.id == creado.persona_id);
    assert_eq!(persona.and_then(|p| p.cuenta_id), Some(creado.cuenta_id));
    assert_eq!(
        asuntos(&unidad),
        vec!["Defina su contraseña de DAD Copropiedad"]
    );
    assert!(
        unidad
            .confirmados()
            .iter()
            .any(|(_, e)| e.accion == ACCION_CREAR)
    );
}

#[tokio::test]
async fn correo_con_cuenta_recibe_el_rol_y_un_aviso_sin_codigo() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let existente = CuentaMemoria::activa("Ana de otra PH", "ana@correo.co");
    let id = existente.id;
    con_cuenta(&unidad, existente);
    let contador = rol(&unidad, &ctx, "contador").await;

    let creado = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ANA@correo.co", contador))
        .await
        .expect("asigna");

    assert!(!creado.cuenta_nueva);
    assert_eq!(creado.cuenta_id, id);
    assert_eq!(unidad.datos().usuarios.cuentas.len(), 1);
    assert!(unidad.datos().usuarios.codigos.is_empty());
    assert_eq!(
        asuntos(&unidad),
        vec!["Tiene un acceso nuevo en DAD Copropiedad"]
    );
}

#[tokio::test]
async fn repetir_el_alta_no_duplica_el_rol_ni_la_persona() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let contador = rol(&unidad, &ctx, "contador").await;
    let primero = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador))
        .await
        .expect("crea");

    let segundo = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador))
        .await
        .expect("reconoce");

    assert!(segundo.asignacion.is_none());
    assert_eq!(segundo.persona_id, primero.persona_id);
    assert_eq!(unidad.datos().asignaciones.len(), 1);
    assert_eq!(unidad.datos().personas.len(), 1);
}

#[tokio::test]
async fn usuario_de_otra_cuenta_es_conflicto_y_no_crea_nada() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let mut otra = CuentaMemoria::activa("Otra", "otra@correo.co");
    otra.usuario = Some(NombreUsuario::new("ana").expect("usuario válido"));
    con_cuenta(&unidad, otra);
    let contador = rol(&unidad, &ctx, "contador").await;
    let mut pedido = nuevo("ana@correo.co", contador);
    pedido.usuario = Some(NombreUsuario::new("ana").expect("usuario válido"));

    let r = crear_usuario(&unidad, &RelojFijo, &ctx, pedido).await;

    assert!(matches!(r, Err(AppError::Conflicto(_))));
    assert_eq!(unidad.datos().usuarios.cuentas.len(), 1);
    assert!(unidad.outbox().is_empty());
}

#[tokio::test]
async fn cuenta_bloqueada_es_conflicto() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let mut bloqueada = CuentaMemoria::activa("Ana", "ana@correo.co");
    bloqueada.estado = EstadoCuenta::Bloqueada;
    con_cuenta(&unidad, bloqueada);
    let contador = rol(&unidad, &ctx, "contador").await;

    let r = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador)).await;

    assert!(matches!(r, Err(AppError::Conflicto(_))));
    assert!(unidad.datos().asignaciones.is_empty());
}

#[tokio::test]
async fn no_crea_un_usuario_con_un_rol_que_el_actor_no_podria_asignar() {
    let unidad = UnidadEnMemoria::default();
    let tenant = TenantId::nuevo();
    let administrador = rol(&unidad, &admin(tenant), "administrador").await;
    let ctx = Contexto::de_cuenta(
        tenant,
        CuentaId::nuevo(),
        [Permiso::RolesAsignar].into_iter().collect(),
    );

    let r = crear_usuario(
        &unidad,
        &RelojFijo,
        &ctx,
        nuevo("ana@correo.co", administrador),
    )
    .await;

    assert!(matches!(r, Err(AppError::SinPermiso(_))));
    assert!(unidad.datos().usuarios.cuentas.is_empty());
    assert!(unidad.outbox().is_empty());
}

#[tokio::test]
async fn sin_roles_asignar_no_crea_usuarios() {
    let unidad = UnidadEnMemoria::default();
    let tenant = TenantId::nuevo();
    let contador = rol(&unidad, &admin(tenant), "contador").await;
    let ctx = Contexto::de_cuenta(
        tenant,
        CuentaId::nuevo(),
        [Permiso::RolesVer].into_iter().collect(),
    );

    let r = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador)).await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::RolesAsignar))
    ));
}

#[tokio::test]
async fn vincula_la_persona_del_censo_con_el_mismo_documento() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let documento = Documento::new("52123456").expect("documento válido");
    let persona = Persona::basica(
        PersonaId::nuevo(),
        TipoPersona::Natural,
        "Ana".into(),
        Some(documento.clone()),
    );
    let persona_id = persona.id;
    unidad.con_datos(|d| d.personas.push(persona));
    let contador = rol(&unidad, &ctx, "contador").await;
    let mut pedido = nuevo("ana@correo.co", contador);
    pedido.documento = Some(documento);

    let creado = crear_usuario(&unidad, &RelojFijo, &ctx, pedido)
        .await
        .expect("crea");

    assert_eq!(creado.persona_id, persona_id);
    assert_eq!(unidad.datos().personas.len(), 1);
    assert_eq!(unidad.datos().personas[0].cuenta_id, Some(creado.cuenta_id));
}

#[tokio::test]
async fn reenvia_el_codigo_solo_a_quien_no_ha_definido_su_contrasena() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    let contador = rol(&unidad, &ctx, "contador").await;
    let pendiente = crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador))
        .await
        .expect("crea");
    let con_clave = CuentaMemoria::activa("Luis", "luis@correo.co");
    let luis = con_clave.id;
    con_cuenta(&unidad, con_clave);
    crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("luis@correo.co", contador))
        .await
        .expect("asigna");

    reenviar_codigo(&unidad, &RelojFijo, &ctx, pendiente.cuenta_id)
        .await
        .expect("reenvía");
    let ya_definida = reenviar_codigo(&unidad, &RelojFijo, &ctx, luis).await;
    let ajena = reenviar_codigo(&unidad, &RelojFijo, &ctx, CuentaId::nuevo()).await;

    assert_eq!(unidad.datos().usuarios.codigos.len(), 2);
    assert!(matches!(ya_definida, Err(AppError::Validacion(_))));
    assert!(matches!(ajena, Err(AppError::NoEncontrado(_))));
}

#[tokio::test]
async fn lista_solo_las_cuentas_con_roles_en_la_entidad() {
    let unidad = UnidadEnMemoria::default();
    let ctx = admin(TenantId::nuevo());
    con_cuenta(&unidad, CuentaMemoria::activa("Sin rol", "nadie@correo.co"));
    let contador = rol(&unidad, &ctx, "contador").await;
    crear_usuario(&unidad, &RelojFijo, &ctx, nuevo("ana@correo.co", contador))
        .await
        .expect("crea");

    let pagina = listar_usuarios(
        &unidad,
        &RelojFijo,
        &ctx,
        &FiltroUsuarios::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect("lista");

    assert_eq!(pagina.datos.len(), 1);
    assert_eq!(pagina.datos[0].roles, vec!["contador".to_owned()]);
    assert!(pagina.datos[0].contrasena_pendiente);
}
