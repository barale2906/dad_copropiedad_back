//! Pruebas funcionales del directorio de comercios y servicios (plan 08, fase 1): la plataforma
//! da de alta un anunciante y su ficha; un propietario lo encuentra por sinónimo y lo califica;
//! las reseñas salen sin autor; un vecino la reporta y la moderación la oculta.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Da de alta «Modistería Rosa» (cobertura nacional) y devuelve su id.
async fn anunciante(p: &Ph) -> String {
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/publicidad/anunciantes",
            Some(&p.root),
            Some(json!({
                "organizacion": {
                    "nit": "900123456-8", "razon_social": "Rosa Pérez",
                    "naturaleza": "natural_con_rut", "tipos": ["otro"]
                },
                "nombre_comercial": "Modistería Rosa",
                "municipio_dane": "11001",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["estado"], "verificado");
    let id = v["id"].as_str().expect("id").to_owned();
    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/ficha"),
            Some(&p.root),
            Some(json!({
                "descripcion": "Arreglos de ropa y confección",
                "categorias": ["costura"],
                "cobertura": "nacional",
                "whatsapp": "+57 300 123 4567",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    id
}

fn sin_autor(v: &Value) {
    let texto = v.to_string();
    assert!(
        !texto.contains("cuenta_id"),
        "la reseña no expone al autor: {texto}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn propietario_encuentra_al_proveedor_y_lo_califica_sin_autor(pool: PgPool) {
    let p = ph(pool).await;
    let id = anunciante(&p).await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/directorio?q=modista", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["datos"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["datos"][0]["anunciante_id"], id.as_str());
    assert_eq!(v["datos"][0]["contacto"]["whatsapp"], "573001234567");
    assert_eq!(v["datos"][0]["destacada"], false);

    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("{}/directorio/anunciantes/{id}/resena", p.base),
            Some(&dueno),
            Some(json!({ "calificacion": 5, "texto": "Muy cumplida" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["propia"], true);
    sin_autor(&v);

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/directorio/anunciantes/{id}/resenas", p.base),
            Some(&p.admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["datos"][0]["texto"], "Muy cumplida");
    assert_eq!(v["datos"][0]["propia"], false, "no es la del administrador");
    sin_autor(&v);

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/directorio/anunciantes/{id}", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["calificacion_promedio"], "5.0");
    assert_eq!(v["resenas"], 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_administrador_ve_el_directorio_pero_no_califica(pool: PgPool) {
    let p = ph(pool).await;
    let id = anunciante(&p).await;

    let (s, v) = p.pedir(Method::GET, "/directorio", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = p
        .pedir(
            Method::PUT,
            &format!("/directorio/anunciantes/{id}/resena"),
            Some(json!({ "calificacion": 1 })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], "sin_permiso");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_categoria_restringida_se_rechaza(pool: PgPool) {
    let p = ph(pool).await;
    let id = anunciante(&p).await;

    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/ficha"),
            Some(&p.root),
            Some(json!({ "categorias": ["tienda", "licores"], "cobertura": "nacional" })),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_destacadas_y_con_el_si(pool: PgPool) {
    let p = ph(pool).await;
    let id = anunciante(&p).await;
    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/ficha"),
            Some(&p.root),
            Some(json!({
                "categorias": ["costura"], "cobertura": "nacional",
                "destacada_hasta": "2099-01-01T00:00:00Z",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (_, sin) = p.pedir(Method::GET, "/directorio/destacadas", None).await;
    assert_eq!(sin, json!([]));
    p.habilitar("publicidad").await;
    let (_, con) = p.pedir(Method::GET, "/directorio/destacadas", None).await;
    assert_eq!(con[0]["anunciante_id"], id.as_str());
    assert_eq!(con[0]["destacada"], true);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_reporte_llega_a_la_moderacion_y_la_resena_se_oculta(pool: PgPool) {
    let p = ph(pool).await;
    let id = anunciante(&p).await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    let (_, resena) = p
        .api
        .pedir(
            Method::PUT,
            &format!("{}/directorio/anunciantes/{id}/resena", p.base),
            Some(&dueno),
            Some(json!({ "calificacion": 1, "texto": "Pésima" })),
        )
        .await;
    let resena_id = resena["id"].as_str().expect("id").to_owned();

    let (s, v) = p
        .pedir(
            Method::POST,
            &format!("/directorio/resenas/{resena_id}/reportes"),
            Some(json!({ "motivo": "Lenguaje ofensivo" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["nuevo"], true);

    let (s, cola) = p
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/publicidad/moderacion/resenas",
            Some(&p.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cola}");
    assert_eq!(cola["datos"][0]["resena_id"], resena_id.as_str());
    assert_eq!(cola["datos"][0]["copropiedad_id"], p.id.as_str());
    let ruta = format!("/api/v1/plataforma/publicidad/moderacion/resenas/{resena_id}");
    let (s, _) = p
        .api
        .pedir(
            Method::POST,
            &ruta,
            Some(&p.root),
            Some(json!({ "copropiedad_id": p.id, "ocultar": true })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "ocultar exige motivo");
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &ruta,
            Some(&p.root),
            Some(json!({ "copropiedad_id": p.id, "ocultar": true, "motivo": "Lenguaje ofensivo" })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");

    let (_, v) = p
        .pedir(
            Method::GET,
            &format!("/directorio/anunciantes/{id}/resenas"),
            None,
        )
        .await;
    assert_eq!(v["datos"], json!([]));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_gestion_del_directorio_es_de_la_plataforma(pool: PgPool) {
    let p = ph(pool).await;

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/publicidad/anunciantes",
            Some(&p.admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, _) = p
        .api
        .pedir(Method::GET, &format!("{}/directorio", p.base), None, None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            "/api/v1/plataforma/publicidad/categorias/arreglo_celulares",
            Some(&p.root),
            Some(json!({
                "padre": "servicios_tecnicos", "nombre": "Arreglo de celulares",
                "sinonimos": ["celular", "pantalla rota", "celular"]
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["sinonimos"], json!(["celular", "pantalla rota"]));
    let (_, v) = p.pedir(Method::GET, "/directorio/categorias", None).await;
    let codigos: Vec<&str> = v
        .as_array()
        .expect("lista")
        .iter()
        .filter_map(|c| c["codigo"].as_str())
        .collect();
    assert!(codigos.contains(&"arreglo_celulares"));
    assert!(
        !codigos.contains(&"licores"),
        "las restringidas no se ofrecen"
    );
}
