//! Integración del censo (plan 07a) sobre PostgreSQL como `dad_app`: casos de uso reales,
//! índices únicos, trigger de propuestas, libros a fecha de corte y RLS.

mod comun;

use application::{
    AppError, Contexto, SolicitudPagina,
    casos_uso::{
        bienes_privados::{NuevoBienPrivado, crear_bien_privado},
        censo::{
            CambiosPersona, NuevaResidencia, NuevoVehiculo, PropuestaCenso, actualizar_persona,
            aprobar_cambio, libro_propietarios, libro_residentes, listar_personas, proponer_cambio,
            registrar_residencia, registrar_vehiculo, terminar_residencia,
        },
        titulares::{
            DatosPersona, RefPersona, SolicitudTitular, registrar_titulares, transferir_titularidad,
        },
    },
    puertos::{RelojSistema, RepositorioCenso, UnidadDeTrabajo},
};
use chrono::Utc;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    BienPrivadoId, CalidadResidencia, ConjuntoPermisos, CuentaId, Documento, NumeroBienPrivado,
    PersonaId, Placa, Porcentaje, TipoBienPrivado, TipoDocumento, TipoPersona, TipoTitularidad,
    TipoVehiculo, UsoBienPrivado,
};
use infra_postgres::{UnidadDeTrabajoPg, transaccion_tenant};
use sqlx::{AssertSqlSafe, PgPool};

struct Entorno {
    duenio: PgPool,
    unidad: UnidadDeTrabajoPg,
    ctx: Contexto,
    bien: BienPrivadoId,
    dueno: PersonaId,
}

fn persona(doc: &str, cuenta: Option<CuentaId>) -> RefPersona {
    RefPersona::Nueva(DatosPersona {
        tipo: TipoPersona::Natural,
        nombre: format!("Persona {doc}"),
        documento: Documento::new(doc).expect("válido"),
        telefono: None,
        correo: None,
        cuenta_id: cuenta,
        tipo_documento: Some(TipoDocumento::Cc),
    })
}

fn titular(p: RefPersona) -> Vec<SolicitudTitular> {
    vec![SolicitudTitular {
        persona: p,
        tipo: TipoTitularidad::Propietario,
        porcentaje: Porcentaje::new(100.into()).expect("válido"),
        es_representante: true,
    }]
}

async fn entorno(pool: PgPool) -> Entorno {
    let ctx = Contexto::new(crear_copropiedad(&pool).await);
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let bien = crear_bien_privado(
        &unidad,
        &ctx,
        NuevoBienPrivado {
            tipo: TipoBienPrivado::Apartamento,
            agrupacion_id: None,
            numero: Some(NumeroBienPrivado::new("101").expect("válido")),
            codigo: None,
            piso: None,
            area_privada_m2: None,
            matricula_inmobiliaria: None,
            cedula_catastral: None,
            uso: UsoBienPrivado::Residencial,
        },
    )
    .await
    .expect("bien")
    .id;
    let dueno = registrar_titulares(
        &unidad,
        &RelojSistema,
        &ctx,
        bien,
        titular(persona("111", None)),
    )
    .await
    .expect("titular")[0]
        .persona_id;
    Entorno {
        duenio: pool,
        unidad,
        ctx,
        bien,
        dueno,
    }
}

fn residencia(p: RefPersona, calidad: CalidadResidencia) -> NuevaResidencia {
    NuevaResidencia {
        persona: p,
        calidad,
        es_responsable: false,
        autorizado_por: None,
        contrato_archivo_id: None,
    }
}

fn carro(placa: &str) -> NuevoVehiculo {
    NuevoVehiculo {
        tipo: TipoVehiculo::Carro,
        placa: Some(Placa::new(placa).expect("válida")),
        marca: None,
        color: None,
        persona_id: None,
        tarjeta_archivo_id: None,
    }
}

async fn crear_cuenta(pool: &PgPool, usuario: &str) -> CuentaId {
    let id = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, $2, $2, 'x', now(), now())",
    )
    .bind(id.as_uuid())
    .bind(usuario)
    .execute(pool)
    .await
    .expect("cuenta");
    id
}

fn sqlstate(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn terminar_la_residencia_vence_el_rol_en_la_bd(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = crear_cuenta(&e.duenio, "hijo").await;
    let r = registrar_residencia(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.bien,
        residencia(persona("222", Some(cuenta)), CalidadResidencia::Familiar),
    )
    .await
    .expect("residencia");
    let vigentes = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM asignaciones_rol a JOIN roles r ON r.id = a.rol_id
             WHERE a.cuenta_id = $1 AND r.codigo = 'residente' AND a.origen = 'residencia'
               AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())",
        )
        .bind(cuenta.as_uuid())
        .fetch_one(&e.duenio)
        .await
        .expect("conteo")
    };
    assert_eq!(vigentes().await, 1);
    terminar_residencia(&e.unidad, &RelojSistema, &e.ctx, r.id)
        .await
        .expect("termina");
    assert_eq!(vigentes().await, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_responsable_por_bien_y_una_placa_vigente(pool: PgPool) {
    let e = entorno(pool).await;
    registrar_residencia(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.bien,
        NuevaResidencia {
            es_responsable: true,
            ..residencia(
                RefPersona::Existente(e.dueno),
                CalidadResidencia::PropietarioResidente,
            )
        },
    )
    .await
    .expect("responsable");
    // La BD repite la regla aunque el caso de uso se saltara.
    let mut tx = transaccion_tenant(&e.duenio, e.ctx.tenant_id)
        .await
        .expect("tx");
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .expect("rol");
    let r = sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO personas (id, tenant_id, nombre) VALUES ('00000000-0000-7000-8000-0000000000aa', '{t}', 'Otro');
         INSERT INTO residencias (id, tenant_id, bien_privado_id, persona_id, calidad, es_responsable, vigente_desde)
         VALUES (gen_random_uuid(), '{t}', '{b}', '00000000-0000-7000-8000-0000000000aa', 'familiar', TRUE, now())",
        t = e.ctx.tenant_id,
        b = e.bien
    )))
    .execute(&mut *tx)
    .await;
    assert!(r.is_err());
    drop(tx);

    registrar_vehiculo(&e.unidad, &RelojSistema, &e.ctx, e.bien, carro("ABC123"))
        .await
        .expect("vehículo");
    assert!(matches!(
        registrar_vehiculo(&e.unidad, &RelojSistema, &e.ctx, e.bien, carro("abc 123")).await,
        Err(AppError::Conflicto(_))
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_propuesta_se_resuelve_una_sola_vez(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = crear_cuenta(&e.duenio, "dueno").await;
    sqlx::query("UPDATE personas SET cuenta_id = $1 WHERE id = $2")
        .bind(cuenta.as_uuid())
        .bind(e.dueno.as_uuid())
        .execute(&e.duenio)
        .await
        .expect("vincula");
    let propietario = Contexto::de_cuenta(e.ctx.tenant_id, cuenta, ConjuntoPermisos::vacio());
    let c = proponer_cambio(
        &e.unidad,
        &RelojSistema,
        &propietario,
        e.bien,
        PropuestaCenso::Vehiculo(carro("XYZ987")),
    )
    .await
    .expect("propone");
    aprobar_cambio(&e.unidad, &RelojSistema, &e.ctx, c.id)
        .await
        .expect("aprueba");
    let mut tx = e.unidad.iniciar(&e.ctx).await.expect("tx");
    let placa = Placa::new("XYZ987").expect("válida");
    assert_eq!(
        tx.vehiculos(None, Some(&placa), false)
            .await
            .expect("lee")
            .len(),
        1
    );
    drop(tx);

    let mut tx = transaccion_tenant(&e.duenio, e.ctx.tenant_id)
        .await
        .expect("tx");
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .expect("rol");
    let r = sqlx::query(AssertSqlSafe(format!(
        "UPDATE cambios_censo SET estado = 'rechazado', motivo = 'x' WHERE id = '{}'",
        c.id
    )))
    .execute(&mut *tx)
    .await;
    assert_eq!(sqlstate(&r.expect_err("una sola vez")), "V0006");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn libros_a_fecha_de_corte(pool: PgPool) {
    let e = entorno(pool).await;
    registrar_residencia(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.bien,
        residencia(
            RefPersona::Existente(e.dueno),
            CalidadResidencia::PropietarioResidente,
        ),
    )
    .await
    .expect("residencia");
    let antes = Utc::now();
    transferir_titularidad(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.bien,
        titular(persona("999", None)),
        None,
    )
    .await
    .expect("transfiere");

    let viejo = libro_propietarios(&e.unidad, &e.ctx, antes)
        .await
        .expect("libro");
    let nuevo = libro_propietarios(&e.unidad, &e.ctx, Utc::now())
        .await
        .expect("libro");
    assert_eq!(viejo.len(), 1);
    assert_eq!(
        viejo[0].documento.as_ref().map(Documento::as_str),
        Some("111")
    );
    assert_eq!(
        nuevo[0].documento.as_ref().map(Documento::as_str),
        Some("999")
    );
    assert_eq!(nuevo[0].codigo.as_str(), "101");
    assert_eq!(nuevo[0].coeficiente, None, "sin cuadro vigente");

    let residentes = libro_residentes(&e.unidad, &e.ctx, Utc::now())
        .await
        .expect("libro");
    assert_eq!(residentes.len(), 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn personas_se_buscan_sin_tildes_y_otra_ph_no_ve_el_censo(pool: PgPool) {
    let e = entorno(pool).await;
    actualizar_persona(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.dueno,
        CambiosPersona {
            nombre: Some("José Álvarez".into()),
            ..Default::default()
        },
    )
    .await
    .expect("actualiza");
    let pagina = listar_personas(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        Some("jose alv".into()),
        SolicitudPagina::primera(),
    )
    .await
    .expect("busca");
    assert_eq!(pagina.datos.len(), 1);
    registrar_vehiculo(&e.unidad, &RelojSistema, &e.ctx, e.bien, carro("QWE456"))
        .await
        .expect("vehículo");

    let otra = Contexto::new(crear_copropiedad(&e.duenio).await);
    let mut tx = e.unidad.iniciar(&otra).await.expect("tx");
    assert!(
        tx.vehiculos(None, None, true)
            .await
            .expect("lee")
            .is_empty()
    );
    assert!(tx.residencias(None, true).await.expect("lee").is_empty());
}
