//! Pruebas de la búsqueda de proveedores, el contacto autorizado y el enlace público (dobles
//! en memoria). La cobertura y el texto contra el SQL real se prueban en
//! `infra-postgres/tests/busqueda_proveedores.rs`.

use chrono::{DateTime, TimeZone, Utc};
use domain::{ConjuntoPermisos, CuentaId, Telefono, TipoPersona};
use rust_decimal::{Decimal, dec};

use super::*;
use crate::puertos::{
    copropiedades::Persona,
    directorio::{Contacto, ResumenCalificacion},
    reloj::RelojFijo,
    unidad_trabajo::memoria::UnidadEnMemoria,
};

const TELEFONO: &str = "573001112233";

fn ahora() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 11, 15, 0, 0)
        .single()
        .expect("fecha")
}

fn reloj() -> RelojFijo {
    RelojFijo(ahora())
}

fn codigo(c: &str) -> CodigoCategoria {
    CodigoCategoria::new(c).expect("válido")
}

fn categoria(c: &str, padre: Option<&str>, nombre: &str, sinonimos: &[&str]) -> Categoria {
    Categoria {
        codigo: codigo(c),
        padre: padre.map(codigo),
        nombre: nombre.into(),
        sinonimos: sinonimos.iter().map(|s| (*s).to_owned()).collect(),
        restringida: false,
        activa: true,
    }
}

fn ficha(
    nombre: &str,
    cat: &str,
    promedio: Option<Decimal>,
    whatsapp: Option<&str>,
) -> FichaDirectorio {
    FichaDirectorio {
        anunciante_id: AnuncianteId::nuevo(),
        nombre_comercial: nombre.into(),
        descripcion: None,
        categorias: vec![codigo(cat)],
        contacto: Contacto {
            whatsapp: whatsapp.map(|w| Telefono::new(w).expect("válido")),
            ..Contacto::default()
        },
        horario: None,
        direccion: None,
        municipio: None,
        destacada: false,
        distancia_km: None,
        calificacion: ResumenCalificacion {
            promedio,
            total: u32::from(promedio.is_some()),
        },
    }
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
    persona: PersonaId,
}

fn entorno(fichas: Vec<FichaDirectorio>) -> Entorno {
    let unidad = UnidadEnMemoria::default();
    let persona = PersonaId::nuevo();
    unidad.con_datos(|d| {
        d.directorio.fichas = fichas;
        d.directorio.categorias = vec![
            categoria("oficios", None, "Oficios", &[]),
            categoria(
                "costura",
                Some("oficios"),
                "Costura y modistería",
                &["costurera", "modista"],
            ),
            categoria("plomeria", None, "Plomería", &["plomero"]),
        ];
        d.directorio.numero_whatsapp = Some("+57 300 000 0000".into());
        d.gestion.linea = Some("LINEA".into());
        let mut p = Persona::basica(persona, TipoPersona::Natural, "Ana Pérez".into(), None);
        p.telefono = Some(Telefono::new(TELEFONO).expect("válido"));
        d.personas.push(p);
    });
    Entorno {
        unidad,
        tenant: TenantId::nuevo(),
        persona,
    }
}

impl Entorno {
    fn ctx(&self) -> Contexto {
        Contexto::new(self.tenant)
    }

    async fn buscar(&self, texto: &str, clave: &str) -> AppResult<ResultadoBusqueda> {
        buscar_proveedores(
            &self.unidad,
            &reloj(),
            &self.ctx(),
            NuevaBusqueda {
                persona_id: self.persona,
                canal: CanalSolicitud::Whatsapp,
                texto: texto.into(),
                clave: clave.into(),
            },
        )
        .await
    }

    async fn autorizar(&self, id: SolicitudProveedorId) -> AppResult<ContactoAutorizado> {
        autorizar_contacto(&self.unidad, &reloj(), &self.ctx(), id, self.persona, None).await
    }

    fn plantillas(&self) -> Vec<(String, String, Vec<String>, Vec<String>)> {
        self.unidad
            .outbox()
            .into_iter()
            .filter_map(|(_, e)| match e.mensaje {
                MensajeOutbox::WhatsApp(MensajeSaliente {
                    telefono,
                    contenido:
                        ContenidoSaliente::Plantilla {
                            nombre,
                            parametros,
                            botones,
                            ..
                        },
                    ..
                }) => Some((nombre, telefono, parametros, botones)),
                _ => None,
            })
            .collect()
    }
}

#[tokio::test]
async fn reconoce_la_categoria_y_sugiere_las_tres_mejores_en_orden() {
    let e = entorno(vec![
        ficha("Costuras Lupe", "costura", Some(dec!(3.0)), None),
        ficha("Modistería Rosa", "costura", Some(dec!(4.9)), None),
        ficha("Arreglos Express", "costura", None, None),
        ficha("Sastrería Don Juan", "costura", Some(dec!(4.2)), None),
        ficha("Plomería Ya", "plomeria", Some(dec!(5.0)), None),
    ]);

    let r = e
        .buscar("¿Alguien conoce una costurera?", "wamid.1")
        .await
        .expect("busca");

    let nombres: Vec<&str> = r
        .sugerencias
        .iter()
        .map(|f| f.nombre_comercial.as_str())
        .collect();
    assert_eq!(
        nombres,
        ["Modistería Rosa", "Sastrería Don Juan", "Costuras Lupe"]
    );
    assert_eq!(r.solicitud.categoria, Some(codigo("costura")));
    assert_eq!(
        r.categoria.map(|c| c.nombre),
        Some("Costura y modistería".into())
    );
    assert_eq!(r.solicitud.sugeridos.len(), 3);
    assert_eq!(e.unidad.datos().directorio.solicitudes.len(), 1);
}

#[tokio::test]
async fn sin_categoria_busca_por_el_texto() {
    let e = entorno(vec![ficha("Panadería La Espiga", "oficios", None, None)]);

    let r = e.buscar("busco la espiga", "w1").await.expect("busca");

    assert_eq!(r.solicitud.categoria, None);
    assert_eq!(r.sugerencias.len(), 1);
}

#[tokio::test]
async fn una_busqueda_sin_resultados_igual_queda_registrada() {
    let e = entorno(vec![]);

    let r = e.buscar("necesito un plomero", "w1").await.expect("busca");

    assert!(r.sugerencias.is_empty());
    assert!(!r.hay_contactables());
    assert_eq!(r.solicitud.categoria, Some(codigo("plomeria")));
    assert_eq!(e.unidad.datos().directorio.solicitudes.len(), 1);
}

#[tokio::test]
async fn reintentar_con_la_misma_clave_no_duplica_la_solicitud() {
    let e = entorno(vec![ficha("Modistería Rosa", "costura", None, None)]);

    let a = e.buscar("busco costurera", "wamid.1").await.expect("busca");
    let b = e.buscar("busco costurera", "wamid.1").await.expect("busca");

    assert_eq!(a.solicitud.id, b.solicitud.id);
    assert_eq!(b.sugerencias.len(), 1);
    assert_eq!(e.unidad.datos().directorio.solicitudes.len(), 1);
    let buscadas = e
        .unidad
        .confirmados()
        .iter()
        .filter(|(_, ev)| ev.accion == ACCION_BUSCAR)
        .count();
    assert_eq!(buscadas, 1);
}

#[tokio::test]
async fn la_auditoria_no_guarda_lo_que_se_busco() {
    let e = entorno(vec![]);

    e.buscar("busco costurera para mi vestido", "w1")
        .await
        .expect("busca");

    let (_, ev) = e
        .unidad
        .confirmados()
        .into_iter()
        .find(|(_, ev)| ev.accion == ACCION_BUSCAR)
        .expect("auditada");
    assert!(!ev.detalle.a_json().to_string().contains("vestido"));
}

#[tokio::test]
async fn la_destacada_solo_va_primero_con_el_modulo_de_publicidad() {
    let mut destacada = ficha("Arreglos Express", "costura", None, None);
    destacada.destacada = true;
    let fichas = vec![
        ficha("Modistería Rosa", "costura", Some(dec!(4.9)), None),
        destacada,
    ];
    let sin = entorno(fichas.clone());
    let con = entorno(fichas);
    con.unidad.habilitar(Modulo::Publicidad);

    let r_sin = sin.buscar("busco costurera", "w1").await.expect("busca");
    let r_con = con.buscar("busco costurera", "w1").await.expect("busca");

    assert_eq!(r_sin.sugerencias[0].nombre_comercial, "Modistería Rosa");
    assert!(r_sin.sugerencias.iter().all(|f| !f.destacada));
    assert_eq!(r_con.sugerencias[0].nombre_comercial, "Arreglos Express");
    assert!(r_con.sugerencias[0].destacada);
}

#[tokio::test]
async fn un_texto_sin_lo_que_se_busca_se_rechaza() {
    let e = entorno(vec![]);

    let r = e.buscar("busco", "w1").await;

    assert!(matches!(r, Err(AppError::Dominio(_))));
    assert!(e.unidad.datos().directorio.solicitudes.is_empty());
}

#[tokio::test]
async fn sin_permiso_del_directorio_no_se_busca() {
    let e = entorno(vec![]);
    let ctx = Contexto::de_cuenta(e.tenant, CuentaId::nuevo(), ConjuntoPermisos::vacio());

    let r = buscar_proveedores(
        &e.unidad,
        &reloj(),
        &ctx,
        NuevaBusqueda {
            persona_id: e.persona,
            canal: CanalSolicitud::App,
            texto: "busco costurera".into(),
            clave: "k".into(),
        },
    )
    .await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::DirectorioVer))
    ));
}

#[tokio::test]
async fn buscar_no_comparte_nada_con_los_proveedores() {
    let e = entorno(vec![ficha(
        "Modistería Rosa",
        "costura",
        None,
        Some("573109998877"),
    )]);

    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    assert!(r.hay_contactables());
    assert!(e.unidad.outbox().is_empty());
    assert!(e.unidad.datos().directorio.contactos.is_empty());
}

#[tokio::test]
async fn autorizar_comparte_el_contacto_solo_con_quienes_reciben_whatsapp() {
    let con_wa = ficha(
        "Modistería Rosa",
        "costura",
        Some(dec!(5)),
        Some("573109998877"),
    );
    let sin_wa = ficha("Costuras Lupe", "costura", Some(dec!(4)), None);
    let (a, b) = (con_wa.anunciante_id, sin_wa.anunciante_id);
    let e = entorno(vec![con_wa, sin_wa]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    let c = e.autorizar(r.solicitud.id).await.expect("autoriza");

    assert_eq!(c.proveedores, ["Modistería Rosa"]);
    assert!(c.seguimiento);
    let datos = e.unidad.datos().directorio;
    assert_eq!(datos.contactos, [(r.solicitud.id, a)]);
    assert!(!datos.contactos.iter().any(|(_, x)| *x == b));
    assert_eq!(
        datos.solicitudes[0].autorizado_en,
        Some(ahora()),
        "la autorización queda registrada"
    );
    let plantillas = e.plantillas();
    assert_eq!(plantillas.len(), 2);
    let (nombre, destino, parametros, _) = &plantillas[0];
    assert_eq!(nombre, PLANTILLA_CONTACTO_PROVEEDOR);
    assert_eq!(destino, "573109998877");
    assert_eq!(
        parametros,
        &[
            "Modistería Rosa".to_owned(),
            "Ana Pérez".into(),
            "costura y modistería".into(),
            format!("+{TELEFONO}"),
        ]
    );
}

#[tokio::test]
async fn el_le_sirvio_se_programa_al_residente_con_su_boton() {
    let f = ficha("Modistería Rosa", "costura", None, Some("573109998877"));
    let e = entorno(vec![f]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    e.autorizar(r.solicitud.id).await.expect("autoriza");

    let datos = e.unidad.datos().directorio;
    let (evento, cuando) = datos
        .encolados
        .iter()
        .find(|(ev, _)| ev.prioridad == PrioridadSalida::Recordatorio)
        .expect("seguimiento");
    assert_eq!(*cuando, Some(ahora() + Duration::days(DIAS_SEGUIMIENTO)));
    let MensajeOutbox::WhatsApp(m) = &evento.mensaje else {
        panic!("por WhatsApp");
    };
    assert_eq!(m.telefono, TELEFONO);
    let ContenidoSaliente::Plantilla {
        nombre, botones, ..
    } = &m.contenido
    else {
        panic!("plantilla");
    };
    assert_eq!(nombre, PLANTILLA_OPINION_PROVEEDOR);
    assert_eq!(
        BotonOpinion::desde_id(&botones[0]),
        Some(BotonOpinion {
            tenant: e.tenant,
            solicitud: r.solicitud.id,
        })
    );
    // El contacto al proveedor no es publicidad: sale con prioridad de evento.
    assert!(
        datos
            .encolados
            .iter()
            .all(|(ev, _)| ev.prioridad != PrioridadSalida::Publicidad)
    );
}

#[tokio::test]
async fn autorizar_dos_veces_no_vuelve_a_enviar() {
    let e = entorno(vec![ficha(
        "Modistería Rosa",
        "costura",
        None,
        Some("573109998877"),
    )]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");
    e.autorizar(r.solicitud.id).await.expect("autoriza");

    let otra = e.autorizar(r.solicitud.id).await.expect("repite");

    assert_eq!(otra.proveedores, ["Modistería Rosa"]);
    assert!(!otra.seguimiento);
    assert_eq!(e.plantillas().len(), 2);
}

#[tokio::test]
async fn nadie_autoriza_la_solicitud_de_otra_persona() {
    let e = entorno(vec![ficha(
        "Modistería Rosa",
        "costura",
        None,
        Some("573109998877"),
    )]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    let otra = autorizar_contacto(
        &e.unidad,
        &reloj(),
        &e.ctx(),
        r.solicitud.id,
        PersonaId::nuevo(),
        None,
    )
    .await;

    assert!(matches!(otra, Err(AppError::NoEncontrado(_))));
    assert!(e.unidad.outbox().is_empty());
}

#[tokio::test]
async fn sin_proveedores_con_whatsapp_no_hay_contacto() {
    let e = entorno(vec![ficha("Costuras Lupe", "costura", None, None)]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    let c = e.autorizar(r.solicitud.id).await;

    assert!(matches!(c, Err(AppError::Validacion(_))));
    assert!(e.unidad.outbox().is_empty());
}

#[tokio::test]
async fn sin_celular_en_el_censo_no_se_comparte_nada() {
    let e = entorno(vec![ficha(
        "Modistería Rosa",
        "costura",
        None,
        Some("573109998877"),
    )]);
    e.unidad.con_datos(|d| d.personas[0].telefono = None);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");

    let c = e.autorizar(r.solicitud.id).await;

    assert!(matches!(c, Err(AppError::Validacion(_))));
    assert!(e.unidad.datos().directorio.contactos.is_empty());
}

#[tokio::test]
async fn proveedores_contactados_lista_los_que_recibieron_el_contacto() {
    let f = ficha("Modistería Rosa", "costura", None, Some("573109998877"));
    let a = f.anunciante_id;
    let e = entorno(vec![f]);
    let r = e.buscar("busco costurera", "w1").await.expect("busca");
    e.autorizar(r.solicitud.id).await.expect("autoriza");

    let v = proveedores_contactados(&e.unidad, &reloj(), &e.ctx(), r.solicitud.id, e.persona)
        .await
        .expect("lista");

    assert_eq!(
        v,
        [ProveedorContactado {
            anunciante_id: a,
            nombre_comercial: "Modistería Rosa".into(),
        }]
    );
}

#[test]
fn boton_de_opinion_ida_y_vuelta() {
    let b = BotonOpinion {
        tenant: TenantId::nuevo(),
        solicitud: SolicitudProveedorId::nuevo(),
    };
    assert_eq!(BotonOpinion::desde_id(&b.id()), Some(b));
    assert_eq!(BotonOpinion::desde_id("vt:x:y:z"), None);
    assert_eq!(BotonOpinion::desde_id("op:no-uuid:otro"), None);
}

// --- Enlace público ---

#[tokio::test]
async fn el_enlace_publico_abre_el_directorio_y_el_chat() {
    let e = entorno(vec![ficha("Modistería Rosa", "costura", None, None)]);

    let enlace = generar_enlace_directorio(&e.unidad, &reloj(), &e.ctx())
        .await
        .expect("genera");
    let d = directorio_publico(
        &e.unidad,
        &reloj(),
        e.tenant,
        &enlace.token,
        FiltroDirectorio::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect("público");

    assert_eq!(
        enlace.whatsapp.as_deref(),
        Some("https://wa.me/573000000000?text=Busco%20")
    );
    assert_eq!(d.fichas.datos.len(), 1);
    assert_eq!(d.whatsapp, enlace.whatsapp);
    assert_eq!(d.copropiedad, "PH en memoria");
    // Solo se guarda la huella del token.
    let guardado = e.unidad.datos().directorio.enlace.expect("enlace");
    assert_eq!(guardado.token_hash.len(), 32);
    assert_ne!(guardado.token_hash, enlace.token.as_bytes());
}

#[tokio::test]
async fn un_token_ajeno_o_reemplazado_no_abre_el_directorio() {
    let e = entorno(vec![]);
    let primero = generar_enlace_directorio(&e.unidad, &reloj(), &e.ctx())
        .await
        .expect("genera");
    let segundo = generar_enlace_directorio(&e.unidad, &reloj(), &e.ctx())
        .await
        .expect("regenera");
    let abrir = |t: String| {
        let unidad = &e.unidad;
        let tenant = e.tenant;
        async move {
            directorio_publico(
                unidad,
                &reloj(),
                tenant,
                &t,
                FiltroDirectorio::default(),
                SolicitudPagina::primera(),
            )
            .await
        }
    };

    assert!(matches!(
        abrir(primero.token).await,
        Err(AppError::NoAutenticado)
    ));
    assert!(matches!(
        abrir("x".repeat(64)).await,
        Err(AppError::NoAutenticado)
    ));
    assert!(abrir(segundo.token.clone()).await.is_ok());

    assert!(
        revocar_enlace_directorio(&e.unidad, &e.ctx())
            .await
            .expect("revoca")
    );
    assert!(matches!(
        abrir(segundo.token).await,
        Err(AppError::NoAutenticado)
    ));
    let estado = estado_enlace_directorio(&e.unidad, &e.ctx())
        .await
        .expect("estado");
    assert_eq!(estado.creado_en, None);
}

#[tokio::test]
async fn solo_quien_puede_compartir_genera_el_enlace() {
    let e = entorno(vec![]);
    let residente = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        [Permiso::DirectorioVer].into_iter().collect(),
    );

    let r = generar_enlace_directorio(&e.unidad, &reloj(), &residente).await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::DirectorioCompartir))
    ));
    assert!(e.unidad.datos().directorio.enlace.is_none());
}
