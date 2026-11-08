//! Pruebas de inventario y mantenimiento con dobles en memoria.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use domain::{
    ArchivoId, CanalTicket, ConjuntoPermisos, CuentaId, Frecuencia, PersonaId, PrioridadTicket,
    TenantId, TipoTicket,
};
use rust_decimal::dec;

use super::*;
use crate::{
    casos_uso::tickets::{NuevoTicket, radicar_ticket},
    puertos::{reloj::RelojFijo, tickets::Responsable, unidad_trabajo::memoria::UnidadEnMemoria},
};

/// Lunes 2 de noviembre de 2026, 9:00 en Bogotá.
fn ahora() -> DateTime<Utc> {
    domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 2, 9, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc)
}

fn hoy() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 2).expect("fecha")
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::GestionAdministrativa);
    Entorno {
        unidad,
        tenant: TenantId::nuevo(),
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

    fn sistema(&self) -> Contexto {
        Contexto::new(self.tenant)
    }

    fn ctx(&self, cuenta: CuentaId, permisos: &[Permiso]) -> Contexto {
        Contexto::de_cuenta(self.tenant, cuenta, permisos.iter().copied().collect())
    }

    async fn activo(&self, codigo: &str) -> Activo {
        registrar_activo(
            &self.unidad,
            &RelojFijo(ahora()),
            &self.admin(),
            NuevoActivo {
                codigo: CodigoActivo::new(codigo).expect("válido"),
                nombre: format!("Equipo {codigo}"),
                clase: ClaseActivo::Equipo,
                marca: None,
                modelo: None,
                serie: None,
                ubicacion: Some("Cuarto de bombas".into()),
                agrupacion_id: None,
                recurso_id: None,
                fecha_compra: None,
                valor: Some(dec!(12500000)),
                garantia_vence: Some(hoy() + Duration::days(10)),
                proveedor_organizacion_id: None,
            },
        )
        .await
        .expect("activo")
    }

    async fn plan_mensual(&self, activo: ActivoId, inicio: NaiveDate) -> PlanMantenimiento {
        guardar_plan(
            &self.unidad,
            &RelojFijo(ahora()),
            &self.admin(),
            None,
            DatosPlan {
                activo_id: Some(activo),
                ubicacion: None,
                actividad: "Revisión mensual".into(),
                recurrencia: Recurrencia::new(Frecuencia::Meses, 1, inicio).expect("válida"),
                anticipacion_dias: 7,
                responsable: None,
                duracion_estimada_min: Some(60),
                checklist: vec!["Presión".into(), "Ruido".into()],
                activo: true,
            },
        )
        .await
        .expect("plan")
    }
}

#[tokio::test]
async fn generar_ordenes_preventivas_es_idempotente_por_plan_y_fecha() {
    let e = entorno();
    let a = e.activo("BOMBA-01").await;
    // Cada mes desde el 5 de octubre: cae el 5 de noviembre (dentro de los 7 días).
    e.plan_mensual(a.id, NaiveDate::from_ymd_opt(2026, 10, 5).expect("fecha"))
        .await;
    let reloj = RelojFijo(ahora());

    let primera = generar_ordenes_preventivas(&e.unidad, &reloj, &e.sistema())
        .await
        .expect("genera");
    let segunda = generar_ordenes_preventivas(&e.unidad, &reloj, &e.sistema())
        .await
        .expect("genera");

    assert_eq!((primera, segunda), (1, 0));
    let ordenes = e.unidad.datos().gestion.ordenes;
    assert_eq!(ordenes.len(), 1);
    let o = &ordenes[0];
    assert_eq!(
        o.programada_para,
        NaiveDate::from_ymd_opt(2026, 11, 5).expect("fecha")
    );
    assert_eq!(o.numero.to_string(), "OT-2026-00001");
    assert_eq!(o.checklist.len(), 2);
    assert_eq!(o.origen, OrigenOrden::PlanPreventivo);
}

#[tokio::test]
async fn un_activo_dado_de_baja_no_genera_ordenes_ni_vuelve_a_operar() {
    let e = entorno();
    let a = e.activo("PLANTA-01").await;
    e.plan_mensual(a.id, hoy()).await;
    let reloj = RelojFijo(ahora());

    let sin_soporte = registrar_movimiento(
        &e.unidad,
        &reloj,
        &e.admin(),
        a.id,
        NuevoMovimiento {
            tipo: TipoMovimiento::Baja,
            ubicacion_destino: None,
            reserva_id: None,
            soporte_archivo_id: None,
            observacion: None,
        },
    )
    .await;
    assert!(matches!(sin_soporte, Err(AppError::Validacion(_))));

    registrar_movimiento(
        &e.unidad,
        &reloj,
        &e.admin(),
        a.id,
        NuevoMovimiento {
            tipo: TipoMovimiento::Baja,
            ubicacion_destino: None,
            reserva_id: None,
            soporte_archivo_id: Some(ArchivoId::nuevo()),
            observacion: Some("Acta de baja 12".into()),
        },
    )
    .await
    .expect("baja");

    let generadas = generar_ordenes_preventivas(&e.unidad, &reloj, &e.sistema())
        .await
        .expect("genera");
    assert_eq!(generadas, 0);
    let r = editar_activo(
        &e.unidad,
        &reloj,
        &e.admin(),
        a.id,
        CambiosActivo {
            estado: Some(EstadoActivo::Operativo),
            ..CambiosActivo::default()
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::Conflicto(_))));
    let hoja = hoja_de_vida(&e.unidad, &reloj, &e.admin(), a.id)
        .await
        .expect("hoja");
    let tipos: Vec<_> = hoja.movimientos.iter().map(|m| m.tipo).collect();
    assert_eq!(tipos, vec![TipoMovimiento::Entrada, TipoMovimiento::Baja]);
}

#[tokio::test]
async fn el_traslado_exige_destino_y_cambia_la_ubicacion() {
    let e = entorno();
    let a = e.activo("TALADRO-1").await;
    let reloj = RelojFijo(ahora());
    let sin_destino = registrar_movimiento(
        &e.unidad,
        &reloj,
        &e.admin(),
        a.id,
        NuevoMovimiento {
            tipo: TipoMovimiento::Traslado,
            ubicacion_destino: None,
            reserva_id: None,
            soporte_archivo_id: None,
            observacion: None,
        },
    )
    .await;
    assert!(matches!(sin_destino, Err(AppError::Validacion(_))));
    registrar_movimiento(
        &e.unidad,
        &reloj,
        &e.admin(),
        a.id,
        NuevoMovimiento {
            tipo: TipoMovimiento::Traslado,
            ubicacion_destino: Some("Bodega 2".into()),
            reserva_id: None,
            soporte_archivo_id: None,
            observacion: None,
        },
    )
    .await
    .expect("traslada");
    let hoja = hoja_de_vida(&e.unidad, &reloj, &e.admin(), a.id)
        .await
        .expect("hoja");
    assert_eq!(hoja.activo.ubicacion.as_deref(), Some("Bodega 2"));
}

#[tokio::test]
async fn el_codigo_de_activo_es_unico() {
    let e = entorno();
    e.activo("BOMBA-01").await;
    let r = registrar_activo(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.admin(),
        NuevoActivo {
            codigo: CodigoActivo::new("bomba-01").expect("válido"),
            nombre: "Otra".into(),
            clase: ClaseActivo::Equipo,
            marca: None,
            modelo: None,
            serie: None,
            ubicacion: None,
            agrupacion_id: None,
            recurso_id: None,
            fecha_compra: None,
            valor: None,
            garantia_vence: None,
            proveedor_organizacion_id: None,
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::Conflicto(_))));
}

#[tokio::test]
async fn verificar_una_correctiva_resuelve_el_ticket_y_avisa_al_solicitante() {
    let e = entorno();
    let a = e.activo("ASC-1").await;
    let reloj = RelojFijo(ahora());
    let ticket = radicar_ticket(
        &e.unidad,
        &reloj,
        &e.sistema(),
        NuevoTicket {
            tipo: TipoTicket::SolicitudMantenimiento,
            categoria_id: None,
            prioridad: Some(PrioridadTicket::Alta),
            canal: CanalTicket::Presencial,
            asunto: "Ascensor con ruido".into(),
            descripcion: "Suena al subir".into(),
            bien_privado_id: None,
            ubicacion: None,
            recurso_id: None,
            activo_id: Some(a.id),
            adjuntos: Vec::new(),
            idempotency_key: "t1".into(),
            solicitante: Some(persona(&e)),
        },
    )
    .await
    .expect("ticket");
    let ejecutor = CuentaId::nuevo();
    let org = domain::OrganizacionId::nuevo();
    e.unidad.con_datos(|d| {
        d.gestion.con_contrato.push(org);
        d.gestion.organizaciones_de.push((ejecutor, org));
    });

    let orden = crear_orden(
        &e.unidad,
        &reloj,
        &e.admin(),
        NuevaOrden {
            tipo: TipoOrden::Correctivo,
            ticket_id: Some(ticket.id),
            activo_id: Some(a.id),
            ubicacion: None,
            descripcion: "Revisar rodamientos".into(),
            asignado: Some(Responsable::Organizacion(org)),
            programada_para: hoy(),
            checklist: vec!["Rodamientos".into()],
        },
    )
    .await
    .expect("orden");
    assert_eq!(orden.origen, OrigenOrden::Ticket);

    // La empresa ve y ejecuta la orden asignada a su organización.
    let ctx_ejecutor = e.ctx(ejecutor, &[Permiso::MantenimientoEjecutar]);
    let visibles = listar_ordenes(
        &e.unidad,
        &reloj,
        &ctx_ejecutor,
        ConsultaOrdenes::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect("lista");
    assert_eq!(visibles.datos.len(), 1);
    avanzar_orden(
        &e.unidad,
        &reloj,
        &ctx_ejecutor,
        orden.id,
        AvanceOrden::Iniciar,
    )
    .await
    .expect("inicia");
    let incompleto = avanzar_orden(
        &e.unidad,
        &reloj,
        &ctx_ejecutor,
        orden.id,
        AvanceOrden::Ejecutar(Ejecucion::default()),
    )
    .await;
    assert!(matches!(incompleto, Err(AppError::Validacion(_))));
    avanzar_orden(
        &e.unidad,
        &reloj,
        &ctx_ejecutor,
        orden.id,
        AvanceOrden::Ejecutar(Ejecucion {
            checklist: vec![PuntoChecklist {
                descripcion: "Rodamientos".into(),
                cumplido: true,
                observacion: None,
            }],
            costo_mano_obra: dec!(150000),
            costo_repuestos: dec!(80000),
            observaciones: Some("Se cambiaron dos rodamientos".into()),
            recibido_por: Some("Portería".into()),
            firma_archivo_id: None,
        }),
    )
    .await
    .expect("ejecuta");

    // Quien ejecuta no verifica.
    let verificar = AvanceOrden::Verificar {
        aprobada: true,
        observacion: Some("Funciona sin ruido".into()),
        resolver_ticket: true,
    };
    let r = avanzar_orden(
        &e.unidad,
        &reloj,
        &ctx_ejecutor,
        orden.id,
        verificar.clone(),
    )
    .await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));
    let o = avanzar_orden(&e.unidad, &reloj, &e.admin(), orden.id, verificar)
        .await
        .expect("verifica");
    assert_eq!(o.estado, EstadoOrden::Verificada);

    let datos = e.unidad.datos();
    let t = datos
        .gestion
        .tickets
        .iter()
        .find(|t| t.id == ticket.id)
        .expect("ticket");
    assert_eq!(t.estado, EstadoTicket::Resuelto);
    assert!(
        datos
            .gestion
            .mensajes
            .iter()
            .any(|m| m.texto.contains("OT-2026-00001"))
    );

    let hoja = hoja_de_vida(&e.unidad, &reloj, &e.admin(), a.id)
        .await
        .expect("hoja");
    assert_eq!(hoja.costo_total, dec!(230000));
}

fn persona(e: &Entorno) -> PersonaId {
    let id = PersonaId::nuevo();
    e.unidad.con_datos(|d| {
        d.personas
            .push(crate::puertos::copropiedades::Persona::basica(
                id,
                domain::TipoPersona::Natural,
                "Carlos Residente".into(),
                None,
            ));
    });
    id
}

#[tokio::test]
async fn quien_solo_ejecuta_no_ve_ordenes_ajenas() {
    let e = entorno();
    let a = e.activo("B-2").await;
    e.plan_mensual(a.id, hoy()).await;
    generar_ordenes_preventivas(&e.unidad, &RelojFijo(ahora()), &e.sistema())
        .await
        .expect("genera");
    let orden = e.unidad.datos().gestion.ordenes[0].id;
    let ajeno = e.ctx(CuentaId::nuevo(), &[Permiso::MantenimientoEjecutar]);
    let r = ver_orden(&e.unidad, &RelojFijo(ahora()), &ajeno, orden).await;
    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
    let r = avanzar_orden(
        &e.unidad,
        &RelojFijo(ahora()),
        &ajeno,
        orden,
        AvanceOrden::Iniciar,
    )
    .await;
    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
}

#[tokio::test]
async fn una_orden_que_nace_de_un_ticket_no_es_preventiva() {
    let e = entorno();
    let r = crear_orden(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.admin(),
        NuevaOrden {
            tipo: TipoOrden::Preventivo,
            ticket_id: Some(domain::TicketId::nuevo()),
            activo_id: None,
            ubicacion: Some("Lobby".into()),
            descripcion: "x".into(),
            asignado: None,
            programada_para: hoy(),
            checklist: Vec::new(),
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn obligacion_sugerida_y_cumplimiento_recalculan_la_proxima_fecha() {
    let e = entorno();
    let reloj = RelojFijo(ahora());
    let o = guardar_obligacion(
        &e.unidad,
        &reloj,
        &e.admin(),
        None,
        DatosObligacion {
            codigo_sugerido: Some("lavado_tanques".into()),
            proxima: Some(hoy() + Duration::days(5)),
            ..DatosObligacion::default()
        },
    )
    .await
    .expect("crea");
    assert_eq!(o.periodicidad_meses, 6);
    assert!(o.nombre.contains("tanques"));

    let repetida = guardar_obligacion(
        &e.unidad,
        &reloj,
        &e.admin(),
        None,
        DatosObligacion {
            codigo_sugerido: Some("lavado_tanques".into()),
            proxima: Some(hoy()),
            ..DatosObligacion::default()
        },
    )
    .await;
    assert!(matches!(repetida, Err(AppError::Conflicto(_))));
    let inexistente = guardar_obligacion(
        &e.unidad,
        &reloj,
        &e.admin(),
        None,
        DatosObligacion {
            codigo_sugerido: Some("no_existe".into()),
            ..DatosObligacion::default()
        },
    )
    .await;
    assert!(matches!(inexistente, Err(AppError::NoEncontrado(_))));

    let alertas = alertas_mantenimiento(&e.unidad, &reloj, &e.admin())
        .await
        .expect("alertas");
    assert_eq!(alertas.obligaciones.len(), 1);

    let futura = registrar_cumplimiento(
        &e.unidad,
        &reloj,
        &e.admin(),
        o.id,
        hoy() + Duration::days(1),
        None,
        None,
    )
    .await;
    assert!(matches!(futura, Err(AppError::Validacion(_))));
    let o = registrar_cumplimiento(&e.unidad, &reloj, &e.admin(), o.id, hoy(), None, None)
        .await
        .expect("cumple");
    assert_eq!(
        o.proxima,
        NaiveDate::from_ymd_opt(2027, 5, 2).expect("fecha")
    );
    let alertas = alertas_mantenimiento(&e.unidad, &reloj, &e.admin())
        .await
        .expect("alertas");
    assert!(alertas.obligaciones.is_empty());
    assert_eq!(alertas.garantias.len(), 0, "sin activos no hay garantías");
}

#[tokio::test]
async fn las_garantias_por_vencer_aparecen_en_las_alertas() {
    let e = entorno();
    e.activo("CCTV-1").await;
    let alertas = alertas_mantenimiento(&e.unidad, &RelojFijo(ahora()), &e.admin())
        .await
        .expect("alertas");
    assert_eq!(alertas.garantias.len(), 1);
}

#[tokio::test]
async fn sin_permiso_de_inventario_no_se_registra() {
    let e = entorno();
    let residente = e.ctx(CuentaId::nuevo(), &[Permiso::TicketsCrear]);
    let r = listar_activos(
        &e.unidad,
        &RelojFijo(ahora()),
        &residente,
        FiltroActivos::default(),
        SolicitudPagina::primera(),
    )
    .await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));
}

struct Directorio(Vec<TenantId>);

impl crate::puertos::DirectorioGestion for Directorio {
    async fn copropiedades_con_gestion_pendiente(&self) -> AppResult<Vec<TenantId>> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn el_worker_de_gestion_recorre_las_copropiedades() {
    let e = entorno();
    let a = e.activo("B-9").await;
    e.plan_mensual(a.id, hoy()).await;
    let r = tareas_gestion(&Directorio(vec![e.tenant]), &e.unidad, &RelojFijo(ahora()))
        .await
        .expect("tareas");
    assert_eq!((r.copropiedades, r.ordenes_generadas), (1, 1));

    // Sin el módulo no falla: simplemente no hace nada.
    e.unidad.con_datos(|d| d.modulos.clear());
    let r = tareas_gestion(&Directorio(vec![e.tenant]), &e.unidad, &RelojFijo(ahora()))
        .await
        .expect("tareas");
    assert_eq!(r.ordenes_generadas, 0);
}

#[tokio::test]
async fn el_tablero_mide_el_cumplimiento_preventivo() {
    let e = entorno();
    let a = e.activo("B-3").await;
    e.plan_mensual(a.id, hoy() - Duration::days(1)).await;
    generar_ordenes_preventivas(&e.unidad, &RelojFijo(ahora()), &e.sistema())
        .await
        .expect("genera");
    let t = tablero_mantenimiento(&e.unidad, &RelojFijo(ahora()), &e.admin())
        .await
        .expect("tablero");
    assert_eq!(
        (
            t.preventivas_programadas,
            t.preventivas_cumplidas,
            t.ordenes_vencidas
        ),
        (1, 0, 1)
    );
    // La orden vencida se puede cancelar con motivo.
    let id = e.unidad.datos().gestion.ordenes[0].id;
    let sin_motivo = avanzar_orden(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.admin(),
        id,
        AvanceOrden::Cancelar { motivo: " ".into() },
    )
    .await;
    assert!(sin_motivo.is_err());
}
