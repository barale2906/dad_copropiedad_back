//! Pruebas de los casos de uso de comunicados con dobles en memoria.

use chrono::{Duration, TimeZone};
use domain::{
    Almacen, ClasificacionArchivo, ClaveArchivo, ConjuntoPermisos, CuentaId, DireccionCorreo,
    NombreArchivo, PersonaId, Telefono, TenantId, TipoArchivo, TipoPersona,
};

use super::*;
use crate::puertos::{
    archivos::{Archivo, memoria::AlmacenEnMemoria},
    comunicados::DestinatarioComunicado,
    copropiedades::Persona,
    reloj::RelojFijo,
    unidad_trabajo::memoria::UnidadEnMemoria,
};

fn ahora() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 15, 0, 0)
        .single()
        .expect("fecha")
}

fn reloj() -> RelojFijo {
    RelojFijo(ahora())
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
    /// Con app y correo.
    ana: (PersonaId, CuentaId),
    /// Solo teléfono.
    beto: PersonaId,
    /// Sin ningún contacto.
    carla: PersonaId,
}

fn destinatario(persona: PersonaId, cuenta: Option<CuentaId>) -> DestinatarioComunicado {
    DestinatarioComunicado {
        persona,
        cuenta,
        correo: None,
        telefono: None,
        tokens: Vec::new(),
    }
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::GestionAdministrativa);
    let tenant = TenantId::nuevo();
    let ana = (PersonaId::nuevo(), CuentaId::nuevo());
    let (beto, carla) = (PersonaId::nuevo(), PersonaId::nuevo());
    unidad.con_datos(|d| {
        let mut p = Persona::basica(ana.0, TipoPersona::Natural, "Ana".into(), None);
        p.cuenta_id = Some(ana.1);
        p.correo = Some(DireccionCorreo::new("ana@correo.co").expect("válido"));
        d.personas.push(p);
        d.gestion.linea = Some("PNID".into());
        d.comunicados.audiencia = vec![
            DestinatarioComunicado {
                correo: Some("ana@correo.co".into()),
                tokens: vec!["tok-1".into(), "tok-2".into()],
                ..destinatario(ana.0, Some(ana.1))
            },
            DestinatarioComunicado {
                telefono: Some(
                    Telefono::new("573001112233")
                        .expect("válido")
                        .as_str()
                        .to_owned(),
                ),
                ..destinatario(beto, None)
            },
            destinatario(carla, None),
        ];
    });
    Entorno {
        unidad,
        tenant,
        ana,
        beto,
        carla,
    }
}

impl Entorno {
    fn admin(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        )
    }

    /// Ana desde la app: sin permisos de gestión.
    fn ana(&self) -> Contexto {
        Contexto::de_cuenta(self.tenant, self.ana.1, ConjuntoPermisos::default())
    }

    async fn borrador(&self, importante: bool, requiere_confirmacion: bool) -> Comunicado {
        crear_comunicado(
            &self.unidad,
            &reloj(),
            &self.admin(),
            nuevo(importante, requiere_confirmacion),
        )
        .await
        .expect("creado")
    }

    async fn enviado(&self, importante: bool, requiere_confirmacion: bool) -> Comunicado {
        let c = self.borrador(importante, requiere_confirmacion).await;
        publicar_comunicado(&self.unidad, &reloj(), &self.admin(), c.id, None)
            .await
            .expect("enviado")
    }

    fn mensajes(&self) -> Vec<MensajeOutbox> {
        self.unidad
            .outbox()
            .into_iter()
            .map(|(_, e)| e.mensaje)
            .collect()
    }
}

fn nuevo(importante: bool, requiere_confirmacion: bool) -> NuevoComunicado {
    NuevoComunicado {
        titulo: "Corte de agua".into(),
        cuerpo: "El jueves no habrá agua de 8 a 12.\nTome precauciones.".into(),
        finalidad: FinalidadTratamiento::Administracion,
        audiencia: Audiencia::Todos,
        importante,
        requiere_confirmacion,
        adjuntos: Vec::new(),
    }
}

#[tokio::test]
async fn crear_exige_permiso_y_modulo() {
    let e = entorno();
    let r = crear_comunicado(&e.unidad, &reloj(), &e.ana(), nuevo(false, false)).await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));

    let sin_modulo = UnidadEnMemoria::default();
    let r = crear_comunicado(&sin_modulo, &reloj(), &e.admin(), nuevo(false, false)).await;
    assert!(matches!(r, Err(AppError::ModuloNoHabilitado(_))));
}

#[tokio::test]
async fn el_cuerpo_admite_saltos_de_linea_pero_no_otros_controles() {
    let e = entorno();
    let c = e.borrador(false, false).await;
    assert!(c.cuerpo.contains('\n'));
    assert_eq!(c.estado, EstadoComunicado::Borrador);

    let mut malo = nuevo(false, false);
    malo.cuerpo = "hola\u{7}".into();
    let r = crear_comunicado(&e.unidad, &reloj(), &e.admin(), malo).await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn publicidad_no_es_finalidad_de_un_comunicado() {
    let e = entorno();
    let mut n = nuevo(false, false);
    n.finalidad = FinalidadTratamiento::Publicidad;
    let r = crear_comunicado(&e.unidad, &reloj(), &e.admin(), n).await;
    assert!(matches!(r, Err(AppError::Dominio(_))));
}

#[tokio::test]
async fn publicar_sin_fecha_envia_por_app_y_correo_sin_whatsapp() {
    let e = entorno();
    let c = e.enviado(false, false).await;

    assert_eq!(c.estado, EstadoComunicado::Enviado);
    assert_eq!(c.enviado_en, Some(ahora()));
    let m = e.mensajes();
    assert_eq!(
        m.iter()
            .filter(|x| matches!(x, MensajeOutbox::Push(_)))
            .count(),
        2,
        "un push por cada dispositivo de Ana"
    );
    assert_eq!(
        m.iter()
            .filter(|x| matches!(x, MensajeOutbox::Correo(_)))
            .count(),
        1
    );
    assert!(
        !m.iter().any(|x| matches!(x, MensajeOutbox::WhatsApp(_))),
        "uno no importante no sale por WhatsApp"
    );
    let resumen = e.unidad.datos().comunicados.entregas.len();
    assert_eq!(resumen, 3, "quedan fijados los tres destinatarios");
}

#[tokio::test]
async fn importante_sale_por_whatsapp_solo_a_quien_no_tiene_app() {
    let e = entorno();
    let c = e.enviado(true, false).await;

    let m = e.mensajes();
    let whatsapp: Vec<_> = m
        .iter()
        .filter_map(|x| match x {
            MensajeOutbox::WhatsApp(w) => Some(w),
            _ => None,
        })
        .collect();
    assert_eq!(whatsapp.len(), 1, "solo Beto, que no tiene app");
    assert_eq!(whatsapp[0].telefono, "573001112233");
    assert!(matches!(
        &whatsapp[0].contenido,
        ContenidoSaliente::Plantilla { nombre, .. } if nombre == PLANTILLA_COMUNICADO
    ));
    // El push de Ana lleva el WhatsApp de respaldo solo si tiene teléfono: no lo tiene.
    assert!(m.iter().all(|x| match x {
        MensajeOutbox::Push(p) =>
            p.respaldo.is_none() && p.datos["comunicado_id"] == c.id.to_string(),
        _ => true,
    }));

    let v = ver_comunicado(&e.unidad, &reloj(), &e.admin(), c.id)
        .await
        .expect("tablero");
    assert_eq!(
        (
            v.resumen.destinatarios,
            v.resumen.por_push,
            v.resumen.por_correo,
            v.resumen.por_whatsapp,
            v.resumen.sin_canal
        ),
        (3, 1, 1, 1, 1)
    );
}

#[tokio::test]
async fn programado_espera_al_worker_y_luego_se_envia() {
    let e = entorno();
    let c = e.borrador(false, false).await;
    let manana = ahora() + Duration::days(1);
    let p = publicar_comunicado(&e.unidad, &reloj(), &e.admin(), c.id, Some(manana))
        .await
        .expect("programado");
    assert_eq!(p.estado, EstadoComunicado::Programado);
    assert!(e.mensajes().is_empty());

    let sistema = Contexto::new(e.tenant);
    let antes = enviar_programados(&e.unidad, &reloj(), &sistema)
        .await
        .expect("worker");
    assert_eq!(antes, 0, "todavía no es la hora");

    let enviados = enviar_programados(&e.unidad, &RelojFijo(manana), &sistema)
        .await
        .expect("worker");
    assert_eq!(enviados, 1);
    assert!(!e.mensajes().is_empty());
    let c = &e.unidad.datos().comunicados.comunicados[0];
    assert_eq!(
        (c.estado, c.enviado_en),
        (EstadoComunicado::Enviado, Some(manana))
    );
}

#[tokio::test]
async fn no_se_programa_a_mas_de_un_anio() {
    let e = entorno();
    let c = e.borrador(false, false).await;
    let r = publicar_comunicado(
        &e.unidad,
        &reloj(),
        &e.admin(),
        c.id,
        Some(ahora() + Duration::days(400)),
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn solo_se_edita_el_borrador_y_lo_enviado_no_se_anula() {
    let e = entorno();
    let c = e.borrador(false, false).await;
    publicar_comunicado(
        &e.unidad,
        &reloj(),
        &e.admin(),
        c.id,
        Some(ahora() + Duration::hours(2)),
    )
    .await
    .expect("programado");
    let r = editar_comunicado(&e.unidad, &reloj(), &e.admin(), c.id, nuevo(true, false)).await;
    assert!(matches!(r, Err(AppError::Conflicto(_))));

    let b = retirar_comunicado(&e.unidad, &reloj(), &e.admin(), c.id)
        .await
        .expect("retirado");
    assert_eq!(
        (b.estado, b.programado_para),
        (EstadoComunicado::Borrador, None)
    );
    let editado = editar_comunicado(&e.unidad, &reloj(), &e.admin(), c.id, nuevo(true, false))
        .await
        .expect("editado");
    assert!(editado.importante);

    let enviado = e.enviado(false, false).await;
    let r = anular_comunicado(&e.unidad, &reloj(), &e.admin(), enviado.id).await;
    assert!(r.is_err(), "lo enviado no se anula");
    let anulado = anular_comunicado(&e.unidad, &reloj(), &e.admin(), c.id)
        .await
        .expect("anulado");
    assert_eq!(anulado.estado, EstadoComunicado::Anulado);
}

#[tokio::test]
async fn abrir_marca_la_lectura_y_entrega_enlaces_de_los_adjuntos() {
    let e = entorno();
    let archivo = ArchivoId::nuevo();
    e.unidad.con_datos(|d| {
        d.archivos.insert(
            archivo,
            Archivo {
                id: archivo,
                clave: ClaveArchivo::new(e.tenant, archivo),
                almacen: Almacen::Local,
                nombre_original: NombreArchivo::new("circular.pdf").expect("válido"),
                tipo: TipoArchivo::Pdf,
                tamano_bytes: 10,
                sha256: "0".repeat(64),
                clasificacion: ClasificacionArchivo::Interno,
                subido_por: None,
                creado_en: ahora(),
            },
        );
    });
    let mut n = nuevo(false, false);
    n.adjuntos = vec![archivo, archivo];
    let c = crear_comunicado(&e.unidad, &reloj(), &e.admin(), n)
        .await
        .expect("creado");
    assert_eq!(c.adjuntos, vec![archivo], "sin adjuntos repetidos");
    publicar_comunicado(&e.unidad, &reloj(), &e.admin(), c.id, None)
        .await
        .expect("enviado");

    let almacen = AlmacenEnMemoria::new(Almacen::Local);
    let abierto = abrir_mi_comunicado(&e.unidad, &reloj(), &almacen, &e.ana(), c.id)
        .await
        .expect("abierto");
    assert_eq!(abierto.recibido.leido_en, Some(ahora()));
    assert_eq!(abierto.adjuntos.len(), 1);
    assert_eq!(abierto.adjuntos[0].tipo_mime, "application/pdf");

    let mias = mis_comunicados(&e.unidad, &reloj(), &e.ana(), SolicitudPagina::primera())
        .await
        .expect("cartelera");
    assert_eq!(mias.datos.len(), 1);
    let v = ver_comunicado(&e.unidad, &reloj(), &e.admin(), c.id)
        .await
        .expect("tablero");
    assert_eq!(v.resumen.leidos, 1);
}

#[tokio::test]
async fn quien_no_es_destinatario_no_lo_ve() {
    let e = entorno();
    let c = e.enviado(false, false).await;
    let otro = Contexto::de_cuenta(e.tenant, CuentaId::nuevo(), ConjuntoPermisos::default());
    let almacen = AlmacenEnMemoria::new(Almacen::Local);
    let r = abrir_mi_comunicado(&e.unidad, &reloj(), &almacen, &otro, c.id).await;
    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
    let vacia = mis_comunicados(&e.unidad, &reloj(), &otro, SolicitudPagina::primera())
        .await
        .expect("vacía");
    assert!(vacia.datos.is_empty());
}

#[tokio::test]
async fn enterado_solo_si_el_comunicado_lo_pide_y_es_idempotente() {
    let e = entorno();
    let sin = e.enviado(false, false).await;
    let r = confirmar_enterado(&e.unidad, &reloj(), &e.ana(), sin.id).await;
    assert!(matches!(r, Err(AppError::Conflicto(_))));

    let con = e.enviado(false, true).await;
    let r = confirmar_enterado(&e.unidad, &reloj(), &e.ana(), con.id)
        .await
        .expect("enterado");
    assert_eq!((r.leido_en, r.enterado_en), (Some(ahora()), Some(ahora())));
    let despues = RelojFijo(ahora() + Duration::hours(1));
    let otra_vez = confirmar_enterado(&e.unidad, &despues, &e.ana(), con.id)
        .await
        .expect("idempotente");
    assert_eq!(otra_vez.enterado_en, Some(ahora()), "conserva el primero");
}

#[tokio::test]
async fn el_envio_queda_auditado_sin_datos_del_contenido() {
    let e = entorno();
    let c = e.enviado(true, false).await;
    let envio = e
        .unidad
        .confirmados()
        .into_iter()
        .map(|(_, ev)| ev)
        .find(|ev| ev.accion == ACCION_ENVIAR)
        .expect("auditado");
    assert_eq!(envio.entidad_id, Some(c.id.as_uuid()));
    let json = envio.detalle.a_json();
    assert_eq!(json["destinatarios"], 3);
    assert!(json.get("titulo").is_none() && json.get("cuerpo").is_none());
    // Sin uso: los demás destinatarios del entorno solo sirven para contar.
    let _ = (e.beto, e.carla);
}
