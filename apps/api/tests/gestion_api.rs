//! Pruebas funcionales de los planes 07b y 07c (tickets, chat lado web, inventario y
//! mantenimiento) sobre el router real y PostgreSQL (rol `dad_app`).

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn ph_con_modulo(pool: PgPool) -> Ph {
    let ph = ph(pool).await;
    ph.habilitar("gestion_administrativa").await;
    ph
}

/// Cuenta nueva con un rol plantilla en la copropiedad; devuelve su token.
async fn con_rol(ph: &Ph, usuario: &str, codigo_rol: &str) -> String {
    let cuenta = ph.api.registrar(usuario).await;
    let (s, roles) = ph.pedir(Method::GET, "/roles", None).await;
    assert_eq!(s, StatusCode::OK, "{roles}");
    let rol = roles
        .as_array()
        .and_then(|r| r.iter().find(|r| r["codigo"] == codigo_rol))
        .and_then(|r| r["id"].as_str())
        .expect("rol plantilla")
        .to_owned();
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/asignaciones",
            Some(json!({ "cuenta_id": cuenta, "rol_id": rol })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    ph.api.token(usuario).await
}

async fn como(
    ph: &Ph,
    token: &str,
    metodo: Method,
    ruta: &str,
    cuerpo: Option<Value>,
) -> (StatusCode, Value) {
    ph.api
        .pedir(metodo, &format!("{}{ruta}", ph.base), Some(token), cuerpo)
        .await
}

async fn outbox_para(ph: &Ph, destino: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT payload->>'asunto' FROM outbox WHERE destino = $1 ORDER BY id")
        .bind(destino)
        .fetch_all(&ph.api.duenio)
        .await
        .expect("outbox")
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_tickets_ni_inventario(pool: PgPool) {
    let ph = ph(pool).await;
    for ruta in [
        "/tickets",
        "/activos",
        "/conversaciones",
        "/ordenes-trabajo",
    ] {
        let (s, v) = ph.pedir(Method::GET, ruta, None).await;
        assert_eq!(
            (s, &v["error"]["codigo"]),
            (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado")),
            "{ruta}"
        );
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reportar_dano_orden_correctiva_verificada_resuelve_y_se_califica(pool: PgPool) {
    let ph = ph_con_modulo(pool).await;
    let (bien, _) = ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;

    // 1. El propietario radica por la web.
    let peticion = json!({
        "tipo": "solicitud_mantenimiento", "prioridad": "alta",
        "asunto": "Ascensor con ruido", "descripcion": "Suena al subir al piso 5",
        "bien_privado_id": bien, "idempotency_key": "web-1"
    });
    let (s, t) = como(
        &ph,
        &dueno,
        Method::POST,
        "/tickets",
        Some(peticion.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(
        t["numero"].as_str().map(|n| n.starts_with("PQR-")),
        Some(true)
    );
    assert_eq!(t["estado"], "abierto");
    let ticket = t["id"].as_str().expect("id").to_owned();
    let (_, repetido) = como(&ph, &dueno, Method::POST, "/tickets", Some(peticion)).await;
    assert_eq!(repetido["id"], t["id"]);
    assert!(
        outbox_para(&ph, "dueno@correo.co")
            .await
            .iter()
            .any(|a| a.contains("Radicamos"))
    );

    // 2. La administración deja una nota interna que el propietario no ve.
    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!("/tickets/{ticket}/mensajes"),
            Some(json!({ "texto": "Llamar al proveedor del ascensor", "interno": true })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, visto) = como(
        &ph,
        &dueno,
        Method::GET,
        &format!("/tickets/{ticket}"),
        None,
    )
    .await;
    assert_eq!(
        visto["mensajes"].as_array().map(Vec::len),
        Some(0),
        "{visto}"
    );
    let (_, staff) = ph
        .pedir(Method::GET, &format!("/tickets/{ticket}"), None)
        .await;
    assert_eq!(staff["mensajes"].as_array().map(Vec::len), Some(1));
    // El propietario no puede escribir notas internas ni rechazar.
    let (s, _) = como(
        &ph,
        &dueno,
        Method::POST,
        &format!("/tickets/{ticket}/mensajes"),
        Some(json!({ "texto": "x", "interno": true })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = como(
        &ph,
        &dueno,
        Method::POST,
        &format!("/tickets/{ticket}/transiciones"),
        Some(json!({ "hacia": "rechazado", "motivo": "no" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // 3. Activo y orden correctiva desde el ticket.
    let (s, activo) = ph
        .pedir(
            Method::POST,
            "/activos",
            Some(
                json!({ "codigo": "asc-1", "nombre": "Ascensor torre 1", "clase": "equipo",
                         "valor": "85000000" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{activo}");
    assert_eq!(activo["codigo"], "ASC-1");
    let hoy = chrono::Utc::now().date_naive().to_string();
    let (s, orden) = ph
        .pedir(
            Method::POST,
            "/ordenes-trabajo",
            Some(
                json!({ "tipo": "correctivo", "ticket_id": ticket, "activo_id": activo["id"],
                         "descripcion": "Revisar rodamientos", "programada_para": hoy,
                         "checklist": ["Rodamientos", "Prueba de carga"] }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{orden}");
    assert_eq!(orden["origen"], "ticket");
    let id_orden = orden["id"].as_str().expect("id").to_owned();
    let avance = |cuerpo: Value| {
        let ruta = format!("/ordenes-trabajo/{id_orden}/avance");
        let ph = &ph;
        async move { ph.pedir(Method::POST, &ruta, Some(cuerpo)).await }
    };
    let (s, _) = avance(json!({ "accion": "iniciar" })).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = avance(json!({ "accion": "ejecutar", "checklist": [] })).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "checklist incompleto: {v}"
    );
    let (s, v) = avance(json!({
        "accion": "ejecutar",
        "checklist": [
            { "descripcion": "Rodamientos", "cumplido": true },
            { "descripcion": "Prueba de carga", "cumplido": true, "observacion": "OK" }
        ],
        "costo_mano_obra": "150000", "costo_repuestos": "80000",
        "observaciones": "Se cambiaron dos rodamientos", "recibido_por": "Portería"
    }))
    .await;
    assert_eq!(
        (s, &v["estado"]),
        (StatusCode::OK, &json!("ejecutada")),
        "{v}"
    );
    let (s, v) = avance(json!({ "accion": "verificar", "aprobada": true,
                                 "observacion": "Sin ruido", "resolver_ticket": true }))
    .await;
    assert_eq!(
        (s, &v["estado"]),
        (StatusCode::OK, &json!("verificada")),
        "{v}"
    );

    // 4. El ticket quedó resuelto, con la respuesta pública y el aviso al propietario.
    let (_, visto) = como(
        &ph,
        &dueno,
        Method::GET,
        &format!("/tickets/{ticket}"),
        None,
    )
    .await;
    assert_eq!(visto["ticket"]["estado"], "resuelto", "{visto}");
    assert!(visto["ticket"]["reabrible_hasta"].is_string());
    assert!(visto["mensajes"].as_array().is_some_and(|m| {
        m.iter()
            .any(|m| m["texto"].as_str().is_some_and(|t| t.contains("OT-")))
    }));
    assert!(
        outbox_para(&ph, "dueno@correo.co")
            .await
            .iter()
            .any(|a| a.contains("resuelto"))
    );

    // 5. Hoja de vida con el costo y encuesta del propietario.
    let (_, hoja) = ph
        .pedir(
            Method::GET,
            &format!("/activos/{}", activo["id"].as_str().expect("id")),
            None,
        )
        .await;
    assert_eq!(hoja["costo_total"], "230000.00");
    assert_eq!(hoja["ordenes"].as_array().map(Vec::len), Some(1));
    let (s, v) = como(
        &ph,
        &dueno,
        Method::POST,
        &format!("/tickets/{ticket}/encuesta"),
        Some(json!({ "calificacion": 5, "comentario": "Rápido" })),
    )
    .await;
    assert_eq!(
        (s, &v["encuesta"]["calificacion"]),
        (StatusCode::OK, &json!(5)),
        "{v}"
    );
    let (s, _) = como(
        &ph,
        &dueno,
        Method::POST,
        &format!("/tickets/{ticket}/encuesta"),
        Some(json!({ "calificacion": 9 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "calificación fuera de rango");

    let (s, tablero) = ph.pedir(Method::GET, "/tickets/tablero", None).await;
    assert_eq!(s, StatusCode::OK, "{tablero}");
    assert!(
        tablero["satisfaccion_promedio"]
            .as_f64()
            .is_some_and(|x| (x - 5.0).abs() < 1e-9)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn convivencia_oculta_al_denunciante_al_asistente_pero_no_al_comite(pool: PgPool) {
    let ph = ph_con_modulo(pool).await;
    let (bien, _) = ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let (s, t) = como(
        &ph,
        &dueno,
        Method::POST,
        "/tickets",
        Some(
            json!({ "tipo": "reporte_convivencia", "asunto": "Ruido nocturno",
                     "descripcion": "Música alta todas las noches", "bien_privado_id": bien,
                     "idempotency_key": "conv-1" }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().expect("id");

    let asistente = con_rol(&ph, "asistente", "asistente_administrativo").await;
    let (s, v) = como(
        &ph,
        &asistente,
        Method::GET,
        &format!("/tickets/{id}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ticket"]["identidad_oculta"], true);
    assert!(
        v["ticket"]["solicitante_persona_id"].is_null() && v["ticket"]["bien_privado_id"].is_null()
    );

    let comite = con_rol(&ph, "comite", "comite_convivencia").await;
    let (_, v) = como(&ph, &comite, Method::GET, &format!("/tickets/{id}"), None).await;
    assert_eq!(v["ticket"]["identidad_oculta"], false);
    assert!(v["ticket"]["solicitante_persona_id"].is_string());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn configuracion_categorias_y_peticiones_invalidas(pool: PgPool) {
    let ph = ph_con_modulo(pool).await;
    let (s, c) = ph.pedir(Method::GET, "/tickets/configuracion", None).await;
    assert_eq!(
        (s, &c["dias_reapertura"]),
        (StatusCode::OK, &json!(5)),
        "{c}"
    );
    let (s, c) = ph
        .pedir(
            Method::PUT,
            "/tickets/configuracion",
            Some(
                json!({ "jornada_inicio": "07:00:00", "jornada_fin": "16:00:00",
                         "incluye_sabado": true, "dias_reapertura": 3,
                         "plazos": [{ "tipo": "queja", "prioridad": null,
                                      "plazo": { "cantidad": 8, "unidad": "dias_habiles" } }] }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["plazos"].as_array().map(Vec::len), Some(1));
    let (s, v) = ph
        .pedir(
            Method::PUT,
            "/tickets/configuracion",
            Some(
                json!({ "jornada_inicio": "17:00:00", "jornada_fin": "08:00:00",
                         "incluye_sabado": false, "dias_reapertura": 3, "plazos": [] }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    let (s, cat) = ph
        .pedir(
            Method::POST,
            "/tickets/categorias",
            Some(json!({ "nombre": "Plomería" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{cat}");
    let (s, _) = ph
        .pedir(
            Method::POST,
            "/tickets/categorias",
            Some(json!({ "nombre": "plomeria" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "nombre repetido sin tildes ni mayúsculas"
    );

    let (s, v) = ph
        .pedir(
            Method::POST,
            "/tickets",
            Some(json!({ "tipo": "queja", "asunto": "x" })),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::BAD_REQUEST, &json!("peticion_invalida"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn preguntas_frecuentes_bandeja_y_obligaciones(pool: PgPool) {
    let ph = ph_con_modulo(pool).await;
    let (s, p) = ph
        .pedir(
            Method::POST,
            "/preguntas-frecuentes",
            Some(json!({ "pregunta": "Horario de la piscina",
                         "respuesta": "Martes a domingo\nde 8:00 a 18:00" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (_, lista) = ph.pedir(Method::GET, "/preguntas-frecuentes", None).await;
    assert_eq!(lista.as_array().map(Vec::len), Some(1));
    let (s, v) = ph
        .pedir(Method::GET, "/conversaciones?estado=abierta", None)
        .await;
    assert_eq!(
        (s, v["datos"].as_array().map(Vec::len)),
        (StatusCode::OK, Some(0))
    );

    let (_, sugeridas) = ph.pedir(Method::GET, "/obligaciones/sugeridas", None).await;
    assert!(
        sugeridas
            .as_array()
            .is_some_and(|s| s.iter().any(|o| o["codigo"] == "recarga_extintores"))
    );
    let hoy = chrono::Utc::now().date_naive();
    let (s, o) = ph
        .pedir(
            Method::POST,
            "/obligaciones",
            Some(json!({ "codigo_sugerido": "recarga_extintores", "proxima": hoy.to_string() })),
        )
        .await;
    assert_eq!(
        (s, &o["periodicidad_meses"]),
        (StatusCode::CREATED, &json!(12)),
        "{o}"
    );
    let (_, alertas) = ph.pedir(Method::GET, "/mantenimiento/alertas", None).await;
    assert_eq!(
        alertas["obligaciones"].as_array().map(Vec::len),
        Some(1),
        "{alertas}"
    );
    let (s, o) = ph
        .pedir(
            Method::POST,
            &format!(
                "/obligaciones/{}/cumplimientos",
                o["id"].as_str().expect("id")
            ),
            Some(json!({ "realizada_el": hoy.to_string(), "observacion": "Certificado 554" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{o}");
    assert!(
        o["proxima"]
            .as_str()
            .is_some_and(|p| p > hoy.to_string().as_str())
    );
    let (_, alertas) = ph.pedir(Method::GET, "/mantenimiento/alertas", None).await;
    assert_eq!(alertas["obligaciones"].as_array().map(Vec::len), Some(0));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn plan_preventivo_y_permisos_del_inventario(pool: PgPool) {
    let ph = ph_con_modulo(pool).await;
    let (_, activo) = ph
        .pedir(
            Method::POST,
            "/activos",
            Some(json!({ "codigo": "BOMBA-01", "nombre": "Bomba 1", "clase": "equipo" })),
        )
        .await;
    let inicio = chrono::Utc::now().date_naive().to_string();
    let (s, plan) = ph
        .pedir(
            Method::POST,
            "/planes-mantenimiento",
            Some(
                json!({ "activo_id": activo["id"], "actividad": "Revisión mensual",
                         "recurrencia": { "frecuencia": "meses", "intervalo": 1, "inicio": inicio },
                         "checklist": ["Presión"] }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{plan}");
    assert_eq!(plan["anticipacion_dias"], 7);
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/planes-mantenimiento",
            Some(json!({ "actividad": "Sin activo ni ubicación",
                         "recurrencia": { "frecuencia": "dias", "intervalo": 1, "inicio": inicio } })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    // Un residente no ve el inventario.
    ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let (s, _) = como(&ph, &dueno, Method::GET, "/activos", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // La baja sin soporte se rechaza.
    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!(
                "/activos/{}/movimientos",
                activo["id"].as_str().expect("id")
            ),
            Some(json!({ "tipo": "baja" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}
