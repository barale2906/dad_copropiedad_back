//! Pruebas de los casos de uso del directorio en la copropiedad (dobles en memoria). Los de
//! plataforma se prueban contra PostgreSQL en `infra-postgres/tests/directorio.rs`.

use chrono::{DateTime, TimeZone, Utc};
use domain::{CodigoCategoria, ConjuntoPermisos};

use super::*;
use crate::puertos::{
    directorio::ResumenCalificacion, reloj::RelojFijo, unidad_trabajo::memoria::UnidadEnMemoria,
};

fn ahora() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 15, 0, 0)
        .single()
        .expect("fecha")
}

fn reloj() -> RelojFijo {
    RelojFijo(ahora())
}

fn costura() -> CodigoCategoria {
    CodigoCategoria::new("costura").expect("válido")
}

fn ficha(nombre: &str, destacada: bool) -> FichaDirectorio {
    FichaDirectorio {
        anunciante_id: AnuncianteId::nuevo(),
        nombre_comercial: nombre.into(),
        descripcion: None,
        categorias: vec![costura()],
        contacto: Contacto::default(),
        horario: None,
        direccion: None,
        municipio: None,
        destacada,
        distancia_km: None,
        calificacion: ResumenCalificacion::default(),
    }
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
    residente: CuentaId,
    basica: AnuncianteId,
    destacada: AnuncianteId,
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    let residente = CuentaId::nuevo();
    let (b, d) = (
        ficha("Modistería Rosa", false),
        ficha("Arreglos Express", true),
    );
    let (basica, destacada) = (b.anunciante_id, d.anunciante_id);
    unidad.con_datos(|datos| {
        datos.directorio.fichas = vec![b, d];
        datos.directorio.residentes.push(residente);
    });
    Entorno {
        unidad,
        tenant: TenantId::nuevo(),
        residente,
        basica,
        destacada,
    }
}

fn permisos(p: &[Permiso]) -> ConjuntoPermisos {
    p.iter().copied().collect()
}

impl Entorno {
    fn ctx_de(&self, cuenta: CuentaId, p: &[Permiso]) -> Contexto {
        Contexto::de_cuenta(self.tenant, cuenta, permisos(p))
    }

    fn ctx_residente(&self) -> Contexto {
        self.ctx_de(
            self.residente,
            &[Permiso::DirectorioVer, Permiso::DirectorioResenar],
        )
    }

    async fn directorio(&self) -> Vec<FichaDirectorio> {
        consultar_directorio(
            &self.unidad,
            &reloj(),
            &self.ctx_residente(),
            FiltroDirectorio::default(),
            SolicitudPagina::primera(),
        )
        .await
        .expect("consulta")
        .datos
    }
}

#[tokio::test]
async fn sin_modulo_de_publicidad_ninguna_ficha_sale_destacada() {
    let e = entorno();

    let fichas = e.directorio().await;
    let destacadas = destacadas(&e.unidad, &reloj(), &e.ctx_residente(), None)
        .await
        .expect("consulta");

    assert_eq!(
        fichas.len(),
        2,
        "el directorio básico funciona sin el módulo"
    );
    assert!(fichas.iter().all(|f| !f.destacada));
    assert!(destacadas.is_empty());
}

#[tokio::test]
async fn con_modulo_de_publicidad_las_destacadas_se_marcan() {
    let e = entorno();
    e.unidad.habilitar(Modulo::Publicidad);

    let fichas = e.directorio().await;
    let destacadas = destacadas(&e.unidad, &reloj(), &e.ctx_residente(), None)
        .await
        .expect("consulta");

    let marcada = fichas.iter().find(|f| f.destacada).expect("una destacada");
    assert_eq!(marcada.anunciante_id, e.destacada);
    assert_eq!(destacadas.len(), 1);
    assert_eq!(destacadas[0].anunciante_id, e.destacada);
}

#[tokio::test]
async fn directorio_exige_permiso() {
    let e = entorno();
    let ctx = e.ctx_de(CuentaId::nuevo(), &[Permiso::TicketsCrear]);

    let r = consultar_directorio(
        &e.unidad,
        &reloj(),
        &ctx,
        FiltroDirectorio::default(),
        SolicitudPagina::primera(),
    )
    .await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::DirectorioVer))
    ));
}

#[tokio::test]
async fn ficha_que_no_alcanza_a_la_ph_no_se_encuentra() {
    let e = entorno();

    let r = ficha_directorio(
        &e.unidad,
        &reloj(),
        &e.ctx_residente(),
        AnuncianteId::nuevo(),
    )
    .await;

    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
}

#[tokio::test]
async fn residente_vigente_resena_y_queda_auditado_sin_texto() {
    let e = entorno();

    let r = resenar(
        &e.unidad,
        &reloj(),
        &e.ctx_residente(),
        e.basica,
        5,
        Some("Muy cumplida con los arreglos"),
    )
    .await
    .expect("reseña");

    assert_eq!(r.estado, EstadoResena::Publicada);
    assert_eq!(e.unidad.datos().directorio.resenas.len(), 1);
    let (_, evento) = e.unidad.confirmados().pop().expect("auditado");
    assert_eq!(evento.accion, ACCION_RESENAR);
    assert!(
        !evento.detalle.a_json().to_string().contains("cumplida"),
        "el texto de la reseña no va a la auditoría"
    );
}

#[tokio::test]
async fn volver_a_resenar_reemplaza_la_resena() {
    let e = entorno();
    let ctx = e.ctx_residente();
    let primera = resenar(&e.unidad, &reloj(), &ctx, e.basica, 2, None)
        .await
        .expect("primera");

    let segunda = resenar(&e.unidad, &reloj(), &ctx, e.basica, 4, Some("Mejoró"))
        .await
        .expect("segunda");

    assert_eq!(primera.id, segunda.id);
    let guardadas = e.unidad.datos().directorio.resenas;
    assert_eq!(guardadas.len(), 1);
    assert_eq!(guardadas[0].calificacion.valor(), 4);
}

#[tokio::test]
async fn quien_no_es_residente_ni_propietario_no_resena() {
    let e = entorno();
    // Un administrador tiene el permiso por plantilla, pero no vive ni es dueño en la PH.
    let ctx = e.ctx_de(
        CuentaId::nuevo(),
        &[Permiso::DirectorioVer, Permiso::DirectorioResenar],
    );

    let r = resenar(&e.unidad, &reloj(), &ctx, e.basica, 5, None).await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::DirectorioResenar))
    ));
    assert!(e.unidad.datos().directorio.resenas.is_empty());
}

#[tokio::test]
async fn no_se_resena_un_anunciante_que_no_alcanza_a_la_ph() {
    let e = entorno();

    let r = resenar(
        &e.unidad,
        &reloj(),
        &e.ctx_residente(),
        AnuncianteId::nuevo(),
        5,
        None,
    )
    .await;

    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
}

#[tokio::test]
async fn calificacion_fuera_de_rango_se_rechaza() {
    let e = entorno();

    let r = resenar(&e.unidad, &reloj(), &e.ctx_residente(), e.basica, 6, None).await;

    assert!(matches!(r, Err(AppError::Dominio(_))));
}

#[tokio::test]
async fn reporte_repetido_no_duplica_y_ocultar_la_saca_del_listado() {
    let e = entorno();
    let resena = resenar(
        &e.unidad,
        &reloj(),
        &e.ctx_residente(),
        e.basica,
        1,
        Some("Grosera"),
    )
    .await
    .expect("reseña");
    let vecino = e.ctx_de(CuentaId::nuevo(), &[Permiso::DirectorioVer]);

    let primero = reportar_resena(&e.unidad, &reloj(), &vecino, resena.id, "Ofensiva")
        .await
        .expect("reporte");
    let repetido = reportar_resena(&e.unidad, &reloj(), &vecino, resena.id, "Ofensiva")
        .await
        .expect("reporte");
    let moderador = ContextoPlataforma::de_cuenta(
        CuentaId::nuevo(),
        permisos(&[Permiso::PlataformaPublicidadModerar]),
        None,
    );
    moderar_resena(
        &e.unidad,
        &reloj(),
        &moderador,
        e.tenant,
        resena.id,
        DecisionModeracion::Ocultar {
            motivo: "Lenguaje ofensivo".into(),
        },
    )
    .await
    .expect("moderada");

    assert!(primero);
    assert!(!repetido);
    let d = e.unidad.datos().directorio;
    assert!(d.reportes.iter().all(|(_, atendido)| atendido.is_some()));
    let visibles = resenas_de_anunciante(
        &e.unidad,
        &reloj(),
        &e.ctx_residente(),
        e.basica,
        SolicitudPagina::primera(),
    )
    .await
    .expect("listado");
    assert!(visibles.datos.is_empty());
    let r = resenar(&e.unidad, &reloj(), &e.ctx_residente(), e.basica, 5, None).await;
    assert!(
        matches!(r, Err(AppError::Conflicto(_))),
        "no se reescribe una oculta"
    );
}

#[tokio::test]
async fn moderar_exige_el_permiso_de_moderacion() {
    let e = entorno();
    let resena = resenar(&e.unidad, &reloj(), &e.ctx_residente(), e.basica, 3, None)
        .await
        .expect("reseña");
    let gestor = ContextoPlataforma::de_cuenta(
        CuentaId::nuevo(),
        permisos(&[Permiso::PlataformaDirectorioGestionar]),
        None,
    );

    let r = moderar_resena(
        &e.unidad,
        &reloj(),
        &gestor,
        e.tenant,
        resena.id,
        DecisionModeracion::Mantener,
    )
    .await;

    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::PlataformaPublicidadModerar))
    ));
}

#[tokio::test]
async fn ocultar_sin_motivo_se_rechaza() {
    let e = entorno();
    let resena = resenar(&e.unidad, &reloj(), &e.ctx_residente(), e.basica, 3, None)
        .await
        .expect("reseña");
    let moderador = ContextoPlataforma::de_cuenta(
        CuentaId::nuevo(),
        permisos(&[Permiso::PlataformaPublicidadModerar]),
        None,
    );

    let r = moderar_resena(
        &e.unidad,
        &reloj(),
        &moderador,
        e.tenant,
        resena.id,
        DecisionModeracion::Ocultar {
            motivo: "  ".into(),
        },
    )
    .await;

    assert!(matches!(r, Err(AppError::Validacion(_))));
}
