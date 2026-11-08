//! Inventario y mantenimiento sobre la transacción del tenant: activos, archivos, movimientos,
//! planes preventivos, órdenes de trabajo, evidencias y obligaciones periódicas.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioMantenimiento,
        mantenimiento::{
            Activo, ArchivoActivo, Cumplimiento, EvidenciaOrden, FiltroActivos, FiltroOrdenes,
            Movimiento, Obligacion, OrdenTrabajo, PlanMantenimiento, TableroMantenimiento,
        },
        tickets::Responsable,
    },
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    ActivoId, AgrupacionId, ArchivoId, ClaseActivo, CodigoActivo, CuentaId, EstadoActivo,
    EstadoOrden, Frecuencia, MomentoEvidencia, MovimientoInventarioId, NumeroOrden, ObligacionId,
    OrdenTrabajoId, OrganizacionId, OrigenOrden, PlanMantenimientoId, PuntoChecklist, Recurrencia,
    RecursoId, ReservaId, TicketId, TipoArchivoActivo, TipoMovimiento, TipoOrden,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPg, copropiedades::error_escritura};

fn entero<T: TryFrom<i64>>(v: i64, campo: &'static str) -> AppResult<T> {
    T::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn patron(texto: &str) -> String {
    let e = texto
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{e}%")
}

struct FilaActivo {
    id: Uuid,
    codigo: String,
    nombre: String,
    clase: String,
    marca: Option<String>,
    modelo: Option<String>,
    serie: Option<String>,
    ubicacion: Option<String>,
    agrupacion_id: Option<Uuid>,
    recurso_id: Option<Uuid>,
    estado: String,
    fecha_compra: Option<NaiveDate>,
    valor: Option<Decimal>,
    garantia_vence: Option<NaiveDate>,
    proveedor_organizacion_id: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaActivo> for Activo {
    type Error = AppError;

    fn try_from(f: FilaActivo) -> AppResult<Self> {
        Ok(Self {
            id: ActivoId::from_uuid(f.id),
            codigo: CodigoActivo::new(&f.codigo)?,
            nombre: f.nombre,
            clase: ClaseActivo::try_from(f.clase.as_str())?,
            marca: f.marca,
            modelo: f.modelo,
            serie: f.serie,
            ubicacion: f.ubicacion,
            agrupacion_id: f.agrupacion_id.map(AgrupacionId::from_uuid),
            recurso_id: f.recurso_id.map(RecursoId::from_uuid),
            estado: EstadoActivo::try_from(f.estado.as_str())?,
            fecha_compra: f.fecha_compra,
            valor: f.valor,
            garantia_vence: f.garantia_vence,
            proveedor_organizacion_id: f.proveedor_organizacion_id.map(OrganizacionId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

struct FilaPlan {
    id: Uuid,
    activo_id: Option<Uuid>,
    ubicacion: Option<String>,
    actividad: String,
    frecuencia: String,
    intervalo: i16,
    inicio: NaiveDate,
    anticipacion_dias: i16,
    responsable_cuenta_id: Option<Uuid>,
    responsable_organizacion_id: Option<Uuid>,
    duracion_estimada_min: Option<i32>,
    checklist: serde_json::Value,
    activo: bool,
}

impl TryFrom<FilaPlan> for PlanMantenimiento {
    type Error = AppError;

    fn try_from(f: FilaPlan) -> AppResult<Self> {
        Ok(Self {
            id: PlanMantenimientoId::from_uuid(f.id),
            activo_id: f.activo_id.map(ActivoId::from_uuid),
            ubicacion: f.ubicacion,
            actividad: f.actividad,
            recurrencia: Recurrencia::new(
                Frecuencia::try_from(f.frecuencia.as_str())?,
                entero(i64::from(f.intervalo), "intervalo")?,
                f.inicio,
            )?,
            anticipacion_dias: entero(i64::from(f.anticipacion_dias), "anticipacion_dias")?,
            responsable: Responsable::desde_columnas(
                f.responsable_cuenta_id.map(CuentaId::from_uuid),
                f.responsable_organizacion_id.map(OrganizacionId::from_uuid),
            ),
            duracion_estimada_min: f
                .duracion_estimada_min
                .map(|m| entero(i64::from(m), "duracion_estimada_min"))
                .transpose()?,
            checklist: serde_json::from_value(f.checklist).map_err(AppError::infra)?,
            activo: f.activo,
        })
    }
}

struct FilaOrden {
    id: Uuid,
    anio: i16,
    consecutivo: i32,
    origen: String,
    tipo: String,
    plan_id: Option<Uuid>,
    ticket_id: Option<Uuid>,
    activo_id: Option<Uuid>,
    ubicacion: Option<String>,
    descripcion: String,
    asignado_cuenta_id: Option<Uuid>,
    asignado_organizacion_id: Option<Uuid>,
    programada_para: NaiveDate,
    estado: String,
    checklist: serde_json::Value,
    costo_mano_obra: Decimal,
    costo_repuestos: Decimal,
    observaciones: Option<String>,
    recibido_por: Option<String>,
    firma_archivo_id: Option<Uuid>,
    iniciada_en: Option<DateTime<Utc>>,
    ejecutada_en: Option<DateTime<Utc>>,
    verificada_en: Option<DateTime<Utc>>,
    verificada_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

impl TryFrom<FilaOrden> for OrdenTrabajo {
    type Error = AppError;

    fn try_from(f: FilaOrden) -> AppResult<Self> {
        Ok(Self {
            id: OrdenTrabajoId::from_uuid(f.id),
            numero: NumeroOrden {
                anio: i32::from(f.anio),
                consecutivo: entero(i64::from(f.consecutivo), "consecutivo")?,
            },
            origen: OrigenOrden::try_from(f.origen.as_str())?,
            tipo: TipoOrden::try_from(f.tipo.as_str())?,
            plan_id: f.plan_id.map(PlanMantenimientoId::from_uuid),
            ticket_id: f.ticket_id.map(TicketId::from_uuid),
            activo_id: f.activo_id.map(ActivoId::from_uuid),
            ubicacion: f.ubicacion,
            descripcion: f.descripcion,
            asignado: Responsable::desde_columnas(
                f.asignado_cuenta_id.map(CuentaId::from_uuid),
                f.asignado_organizacion_id.map(OrganizacionId::from_uuid),
            ),
            programada_para: f.programada_para,
            estado: EstadoOrden::try_from(f.estado.as_str())?,
            checklist: serde_json::from_value::<Vec<PuntoChecklist>>(f.checklist)
                .map_err(AppError::infra)?,
            costo_mano_obra: f.costo_mano_obra,
            costo_repuestos: f.costo_repuestos,
            observaciones: f.observaciones,
            recibido_por: f.recibido_por,
            firma_archivo_id: f.firma_archivo_id.map(ArchivoId::from_uuid),
            iniciada_en: f.iniciada_en,
            ejecutada_en: f.ejecutada_en,
            verificada_en: f.verificada_en,
            verificada_por: f.verificada_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

struct FilaObligacion {
    id: Uuid,
    codigo_sugerido: Option<String>,
    nombre: String,
    descripcion: Option<String>,
    periodicidad_meses: i16,
    proxima: NaiveDate,
    dias_alerta: i16,
    activo_id: Option<Uuid>,
    responsable_organizacion_id: Option<Uuid>,
    activa: bool,
}

impl TryFrom<FilaObligacion> for Obligacion {
    type Error = AppError;

    fn try_from(f: FilaObligacion) -> AppResult<Self> {
        Ok(Self {
            id: ObligacionId::from_uuid(f.id),
            codigo_sugerido: f.codigo_sugerido,
            nombre: f.nombre,
            descripcion: f.descripcion,
            periodicidad_meses: entero(i64::from(f.periodicidad_meses), "periodicidad_meses")?,
            proxima: f.proxima,
            dias_alerta: entero(i64::from(f.dias_alerta), "dias_alerta")?,
            activo_id: f.activo_id.map(ActivoId::from_uuid),
            responsable_organizacion_id: f
                .responsable_organizacion_id
                .map(OrganizacionId::from_uuid),
            activa: f.activa,
        })
    }
}

impl RepositorioMantenimiento for TransaccionPg {
    async fn activos(
        &mut self,
        f: &FiltroActivos,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Activo>> {
        let filas = sqlx::query_as!(
            FilaActivo,
            r#"SELECT id, codigo, nombre, clase, marca, modelo, serie, ubicacion, agrupacion_id,
                      recurso_id, estado, fecha_compra, valor, garantia_vence,
                      proveedor_organizacion_id, creado_en
               FROM activos
               WHERE ($1::text IS NULL OR clase = $1)
                 AND ($2::text IS NULL OR estado = $2)
                 AND ($3::text IS NULL OR normalizar(nombre) LIKE normalizar($3) ESCAPE '\'
                      OR codigo LIKE upper($3) ESCAPE '\')
                 AND ($4::uuid IS NULL OR id > $4)
               ORDER BY id
               LIMIT $5"#,
            f.clase.map(ClaseActivo::as_str),
            f.estado.map(EstadoActivo::as_str),
            f.texto.as_deref().map(patron),
            pagina.despues(),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Activo::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |a| a.id.as_uuid()))
    }

    async fn activo(&mut self, id: ActivoId) -> AppResult<Option<Activo>> {
        sqlx::query_as!(
            FilaActivo,
            "SELECT id, codigo, nombre, clase, marca, modelo, serie, ubicacion, agrupacion_id,
                    recurso_id, estado, fecha_compra, valor, garantia_vence,
                    proveedor_organizacion_id, creado_en
             FROM activos WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Activo::try_from)
        .transpose()
    }

    async fn insertar_activo(&mut self, a: &Activo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO activos (id, tenant_id, codigo, nombre, clase, marca, modelo, serie,
                                  ubicacion, agrupacion_id, recurso_id, estado, fecha_compra,
                                  valor, garantia_vence, proveedor_organizacion_id, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.codigo.as_str(),
            a.nombre,
            a.clase.as_str(),
            a.marca,
            a.modelo,
            a.serie,
            a.ubicacion,
            a.agrupacion_id.map(AgrupacionId::as_uuid),
            a.recurso_id.map(RecursoId::as_uuid),
            a.estado.as_str(),
            a.fecha_compra,
            a.valor,
            a.garantia_vence,
            a.proveedor_organizacion_id.map(OrganizacionId::as_uuid),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya existe un activo con ese código",
                "agrupación, recurso u organización",
            )
        })?;
        Ok(())
    }

    async fn actualizar_activo(&mut self, a: &Activo) -> AppResult<()> {
        sqlx::query!(
            "UPDATE activos SET nombre = $2, marca = $3, modelo = $4, serie = $5, ubicacion = $6,
                    recurso_id = $7, estado = $8, valor = $9, garantia_vence = $10,
                    proveedor_organizacion_id = $11, actualizado_en = now()
             WHERE id = $1",
            a.id.as_uuid(),
            a.nombre,
            a.marca,
            a.modelo,
            a.serie,
            a.ubicacion,
            a.recurso_id.map(RecursoId::as_uuid),
            a.estado.as_str(),
            a.valor,
            a.garantia_vence,
            a.proveedor_organizacion_id.map(OrganizacionId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "activo inválido", "recurso u organización"))?;
        Ok(())
    }

    async fn insertar_archivo_activo(
        &mut self,
        activo: ActivoId,
        a: &ArchivoActivo,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO activo_archivos (tenant_id, activo_id, archivo_id, tipo) VALUES ($1, $2, $3, $4)",
            self.ctx.tenant_id.as_uuid(),
            activo.as_uuid(),
            a.archivo_id.as_uuid(),
            a.tipo.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el archivo ya está en la hoja de vida", "archivo"))?;
        Ok(())
    }

    async fn archivos_activo(&mut self, activo: ActivoId) -> AppResult<Vec<ArchivoActivo>> {
        sqlx::query!(
            "SELECT archivo_id, tipo FROM activo_archivos WHERE activo_id = $1 ORDER BY creado_en",
            activo.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(ArchivoActivo {
                archivo_id: ArchivoId::from_uuid(r.archivo_id),
                tipo: TipoArchivoActivo::try_from(r.tipo.as_str())?,
            })
        })
        .collect()
    }

    async fn insertar_movimiento(&mut self, m: &Movimiento) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO movimientos_inventario (id, tenant_id, activo_id, tipo, ubicacion_destino,
                                                 reserva_id, soporte_archivo_id, observacion,
                                                 registrado_por, ocurrido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            m.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            m.activo_id.as_uuid(),
            m.tipo.as_str(),
            m.ubicacion_destino,
            m.reserva_id.map(ReservaId::as_uuid),
            m.soporte_archivo_id.map(ArchivoId::as_uuid),
            m.observacion,
            m.registrado_por.map(CuentaId::as_uuid),
            m.ocurrido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "movimiento repetido", "activo, reserva o archivo"))?;
        Ok(())
    }

    async fn movimientos(&mut self, activo: ActivoId) -> AppResult<Vec<Movimiento>> {
        sqlx::query!(
            "SELECT id, tipo, ubicacion_destino, reserva_id, soporte_archivo_id, observacion,
                    registrado_por, ocurrido_en
             FROM movimientos_inventario WHERE activo_id = $1 ORDER BY ocurrido_en, id",
            activo.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(Movimiento {
                id: MovimientoInventarioId::from_uuid(r.id),
                activo_id: activo,
                tipo: TipoMovimiento::try_from(r.tipo.as_str())?,
                ubicacion_destino: r.ubicacion_destino,
                reserva_id: r.reserva_id.map(ReservaId::from_uuid),
                soporte_archivo_id: r.soporte_archivo_id.map(ArchivoId::from_uuid),
                observacion: r.observacion,
                registrado_por: r.registrado_por.map(CuentaId::from_uuid),
                ocurrido_en: r.ocurrido_en,
            })
        })
        .collect()
    }

    async fn planes(&mut self, solo_activos: bool) -> AppResult<Vec<PlanMantenimiento>> {
        sqlx::query_as!(
            FilaPlan,
            "SELECT id, activo_id, ubicacion, actividad, frecuencia, intervalo, inicio,
                    anticipacion_dias, responsable_cuenta_id, responsable_organizacion_id,
                    duracion_estimada_min, checklist, activo
             FROM planes_mantenimiento WHERE (NOT $1 OR activo) ORDER BY id",
            solo_activos,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(PlanMantenimiento::try_from)
        .collect()
    }

    async fn plan(&mut self, id: PlanMantenimientoId) -> AppResult<Option<PlanMantenimiento>> {
        sqlx::query_as!(
            FilaPlan,
            "SELECT id, activo_id, ubicacion, actividad, frecuencia, intervalo, inicio,
                    anticipacion_dias, responsable_cuenta_id, responsable_organizacion_id,
                    duracion_estimada_min, checklist, activo
             FROM planes_mantenimiento WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(PlanMantenimiento::try_from)
        .transpose()
    }

    async fn insertar_plan(&mut self, p: &PlanMantenimiento) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(p.responsable);
        sqlx::query!(
            "INSERT INTO planes_mantenimiento (id, tenant_id, activo_id, ubicacion, actividad,
                                               frecuencia, intervalo, inicio, anticipacion_dias,
                                               responsable_cuenta_id, responsable_organizacion_id,
                                               duracion_estimada_min, checklist, activo)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.activo_id.map(ActivoId::as_uuid),
            p.ubicacion,
            p.actividad,
            p.recurrencia.frecuencia.as_str(),
            entero::<i16>(i64::from(p.recurrencia.intervalo), "intervalo")?,
            p.recurrencia.inicio,
            entero::<i16>(i64::from(p.anticipacion_dias), "anticipacion_dias")?,
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            p.duracion_estimada_min
                .map(|m| entero::<i32>(i64::from(m), "duracion"))
                .transpose()?,
            serde_json::to_value(&p.checklist).map_err(AppError::infra)?,
            p.activo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "plan repetido", "activo u organización"))?;
        Ok(())
    }

    async fn actualizar_plan(&mut self, p: &PlanMantenimiento) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(p.responsable);
        sqlx::query!(
            "UPDATE planes_mantenimiento SET activo_id = $2, ubicacion = $3, actividad = $4,
                    frecuencia = $5, intervalo = $6, inicio = $7, anticipacion_dias = $8,
                    responsable_cuenta_id = $9, responsable_organizacion_id = $10,
                    duracion_estimada_min = $11, checklist = $12, activo = $13,
                    actualizado_en = now()
             WHERE id = $1",
            p.id.as_uuid(),
            p.activo_id.map(ActivoId::as_uuid),
            p.ubicacion,
            p.actividad,
            p.recurrencia.frecuencia.as_str(),
            entero::<i16>(i64::from(p.recurrencia.intervalo), "intervalo")?,
            p.recurrencia.inicio,
            entero::<i16>(i64::from(p.anticipacion_dias), "anticipacion_dias")?,
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            p.duracion_estimada_min
                .map(|m| entero::<i32>(i64::from(m), "duracion"))
                .transpose()?,
            serde_json::to_value(&p.checklist).map_err(AppError::infra)?,
            p.activo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "plan inválido", "activo u organización"))?;
        Ok(())
    }

    async fn ordenes(
        &mut self,
        f: &FiltroOrdenes,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<OrdenTrabajo>> {
        let (cuenta, orgs) = match &f.asignadas_a {
            Some((c, o)) => (Some(c.as_uuid()), o.iter().map(|x| x.as_uuid()).collect()),
            None => (None, Vec::<Uuid>::new()),
        };
        let filas = sqlx::query_as!(
            FilaOrden,
            "SELECT id, anio, consecutivo, origen, tipo, plan_id, ticket_id, activo_id, ubicacion,
                    descripcion, asignado_cuenta_id, asignado_organizacion_id, programada_para,
                    estado, checklist, costo_mano_obra, costo_repuestos, observaciones,
                    recibido_por, firma_archivo_id, iniciada_en, ejecutada_en, verificada_en,
                    verificada_por, creado_en, actualizado_en
             FROM ordenes_trabajo
             WHERE ($1::text IS NULL OR estado = $1)
               AND ($2::uuid IS NULL OR activo_id = $2)
               AND ($3::uuid IS NULL OR ticket_id = $3)
               AND (NOT $4 OR asignado_cuenta_id = $5 OR asignado_organizacion_id = ANY($6))
               AND ($7::date IS NULL OR (programada_para < $7
                                          AND estado IN ('programada', 'en_ejecucion')))
               AND ($8::uuid IS NULL OR id < $8)
             ORDER BY id DESC
             LIMIT $9",
            f.estado.map(EstadoOrden::as_str),
            f.activo_id.map(ActivoId::as_uuid),
            f.ticket_id.map(TicketId::as_uuid),
            f.asignadas_a.is_some(),
            cuenta,
            &orgs,
            f.vencidas_a,
            pagina.despues(),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(OrdenTrabajo::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |o| o.id.as_uuid()))
    }

    async fn orden(&mut self, id: OrdenTrabajoId) -> AppResult<Option<OrdenTrabajo>> {
        sqlx::query_as!(
            FilaOrden,
            "SELECT id, anio, consecutivo, origen, tipo, plan_id, ticket_id, activo_id, ubicacion,
                    descripcion, asignado_cuenta_id, asignado_organizacion_id, programada_para,
                    estado, checklist, costo_mano_obra, costo_repuestos, observaciones,
                    recibido_por, firma_archivo_id, iniciada_en, ejecutada_en, verificada_en,
                    verificada_por, creado_en, actualizado_en
             FROM ordenes_trabajo WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(OrdenTrabajo::try_from)
        .transpose()
    }

    async fn existe_orden_de_plan(
        &mut self,
        plan: PlanMantenimientoId,
        fecha: NaiveDate,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM ordenes_trabajo
                              WHERE plan_id = $1 AND programada_para = $2) AS "existe!""#,
            plan.as_uuid(),
            fecha,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn insertar_orden(&mut self, o: &OrdenTrabajo) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(o.asignado);
        sqlx::query!(
            "INSERT INTO ordenes_trabajo (id, tenant_id, anio, consecutivo, origen, tipo, plan_id,
                                          ticket_id, activo_id, ubicacion, descripcion,
                                          asignado_cuenta_id, asignado_organizacion_id,
                                          programada_para, estado, checklist, creado_en,
                                          actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18)",
            o.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            entero::<i16>(i64::from(o.numero.anio), "anio")?,
            entero::<i32>(i64::from(o.numero.consecutivo), "consecutivo")?,
            o.origen.as_str(),
            o.tipo.as_str(),
            o.plan_id.map(PlanMantenimientoId::as_uuid),
            o.ticket_id.map(TicketId::as_uuid),
            o.activo_id.map(ActivoId::as_uuid),
            o.ubicacion,
            o.descripcion,
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            o.programada_para,
            o.estado.as_str(),
            serde_json::to_value(&o.checklist).map_err(AppError::infra)?,
            o.creado_en,
            o.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya existe la orden de ese plan para esa fecha",
                "plan, ticket, activo u organización",
            )
        })?;
        Ok(())
    }

    async fn actualizar_orden(&mut self, o: &OrdenTrabajo) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(o.asignado);
        sqlx::query!(
            "UPDATE ordenes_trabajo SET asignado_cuenta_id = $2, asignado_organizacion_id = $3,
                    programada_para = $4, estado = $5, checklist = $6, costo_mano_obra = $7,
                    costo_repuestos = $8, observaciones = $9, recibido_por = $10,
                    firma_archivo_id = $11, iniciada_en = $12, ejecutada_en = $13,
                    verificada_en = $14, verificada_por = $15, actualizado_en = $16
             WHERE id = $1",
            o.id.as_uuid(),
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            o.programada_para,
            o.estado.as_str(),
            serde_json::to_value(&o.checklist).map_err(AppError::infra)?,
            o.costo_mano_obra,
            o.costo_repuestos,
            o.observaciones,
            o.recibido_por,
            o.firma_archivo_id.map(ArchivoId::as_uuid),
            o.iniciada_en,
            o.ejecutada_en,
            o.verificada_en,
            o.verificada_por.map(CuentaId::as_uuid),
            o.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "orden inválida", "archivo u organización"))?;
        Ok(())
    }

    async fn insertar_evidencia(
        &mut self,
        orden: OrdenTrabajoId,
        e: &EvidenciaOrden,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO orden_evidencias (tenant_id, orden_id, archivo_id, momento) VALUES ($1, $2, $3, $4)",
            self.ctx.tenant_id.as_uuid(),
            orden.as_uuid(),
            e.archivo_id.as_uuid(),
            e.momento.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "la evidencia ya está en la orden", "archivo"))?;
        Ok(())
    }

    async fn evidencias(&mut self, orden: OrdenTrabajoId) -> AppResult<Vec<EvidenciaOrden>> {
        sqlx::query!(
            "SELECT archivo_id, momento FROM orden_evidencias WHERE orden_id = $1 ORDER BY creado_en",
            orden.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(EvidenciaOrden {
                archivo_id: ArchivoId::from_uuid(r.archivo_id),
                momento: MomentoEvidencia::try_from(r.momento.as_str())?,
            })
        })
        .collect()
    }

    async fn obligaciones(&mut self, solo_activas: bool) -> AppResult<Vec<Obligacion>> {
        sqlx::query_as!(
            FilaObligacion,
            "SELECT id, codigo_sugerido, nombre, descripcion, periodicidad_meses, proxima,
                    dias_alerta, activo_id, responsable_organizacion_id, activa
             FROM obligaciones_periodicas WHERE (NOT $1 OR activa) ORDER BY proxima, id",
            solo_activas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Obligacion::try_from)
        .collect()
    }

    async fn obligacion(&mut self, id: ObligacionId) -> AppResult<Option<Obligacion>> {
        sqlx::query_as!(
            FilaObligacion,
            "SELECT id, codigo_sugerido, nombre, descripcion, periodicidad_meses, proxima,
                    dias_alerta, activo_id, responsable_organizacion_id, activa
             FROM obligaciones_periodicas WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Obligacion::try_from)
        .transpose()
    }

    async fn insertar_obligacion(&mut self, o: &Obligacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO obligaciones_periodicas (id, tenant_id, codigo_sugerido, nombre,
                                                  descripcion, periodicidad_meses, proxima,
                                                  dias_alerta, activo_id,
                                                  responsable_organizacion_id, activa)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            o.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            o.codigo_sugerido,
            o.nombre,
            o.descripcion,
            entero::<i16>(i64::from(o.periodicidad_meses), "periodicidad_meses")?,
            o.proxima,
            entero::<i16>(i64::from(o.dias_alerta), "dias_alerta")?,
            o.activo_id.map(ActivoId::as_uuid),
            o.responsable_organizacion_id.map(OrganizacionId::as_uuid),
            o.activa,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "esa obligación sugerida ya está activa",
                "activo u organización",
            )
        })?;
        Ok(())
    }

    async fn actualizar_obligacion(&mut self, o: &Obligacion) -> AppResult<()> {
        sqlx::query!(
            "UPDATE obligaciones_periodicas SET nombre = $2, descripcion = $3,
                    periodicidad_meses = $4, proxima = $5, dias_alerta = $6, activo_id = $7,
                    responsable_organizacion_id = $8, activa = $9, actualizado_en = now()
             WHERE id = $1",
            o.id.as_uuid(),
            o.nombre,
            o.descripcion,
            entero::<i16>(i64::from(o.periodicidad_meses), "periodicidad_meses")?,
            o.proxima,
            entero::<i16>(i64::from(o.dias_alerta), "dias_alerta")?,
            o.activo_id.map(ActivoId::as_uuid),
            o.responsable_organizacion_id.map(OrganizacionId::as_uuid),
            o.activa,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "esa obligación sugerida ya está activa",
                "activo u organización",
            )
        })?;
        Ok(())
    }

    async fn insertar_cumplimiento(&mut self, c: &Cumplimiento) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cumplimientos_obligacion (id, tenant_id, obligacion_id, realizada_el,
                                                   soporte_archivo_id, observacion,
                                                   registrado_por, registrado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            c.obligacion_id.as_uuid(),
            c.realizada_el,
            c.soporte_archivo_id.map(ArchivoId::as_uuid),
            c.observacion,
            c.registrado_por.map(CuentaId::as_uuid),
            c.registrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "cumplimiento repetido", "obligación o archivo"))?;
        Ok(())
    }

    async fn cumplimientos(&mut self, obligacion: ObligacionId) -> AppResult<Vec<Cumplimiento>> {
        Ok(sqlx::query!(
            "SELECT realizada_el, soporte_archivo_id, observacion, registrado_por, registrado_en
             FROM cumplimientos_obligacion WHERE obligacion_id = $1
             ORDER BY realizada_el DESC, id DESC",
            obligacion.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| Cumplimiento {
            obligacion_id: obligacion,
            realizada_el: r.realizada_el,
            soporte_archivo_id: r.soporte_archivo_id.map(ArchivoId::from_uuid),
            observacion: r.observacion,
            registrado_por: r.registrado_por.map(CuentaId::from_uuid),
            registrado_en: r.registrado_en,
        })
        .collect())
    }

    async fn tablero_mantenimiento(&mut self, hoy: NaiveDate) -> AppResult<TableroMantenimiento> {
        let desde = hoy - Duration::days(365);
        let r = sqlx::query!(
            r#"SELECT
                 count(*) FILTER (WHERE tipo = 'preventivo' AND estado <> 'cancelada'
                                    AND programada_para BETWEEN $1 AND $2) AS "programadas!",
                 count(*) FILTER (WHERE tipo = 'preventivo'
                                    AND estado IN ('ejecutada', 'verificada')
                                    AND programada_para BETWEEN $1 AND $2) AS "cumplidas!",
                 count(*) FILTER (WHERE estado IN ('programada', 'en_ejecucion')
                                    AND programada_para < $2) AS "vencidas!"
               FROM ordenes_trabajo"#,
            desde,
            hoy,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let costos = sqlx::query!(
            r#"SELECT a.id, a.nombre,
                      sum(o.costo_mano_obra + o.costo_repuestos) AS "costo!"
               FROM ordenes_trabajo o JOIN activos a ON a.id = o.activo_id
               WHERE o.estado <> 'cancelada'
               GROUP BY a.id, a.nombre
               HAVING sum(o.costo_mano_obra + o.costo_repuestos) > 0
               ORDER BY 3 DESC, a.id
               LIMIT 20"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|c| (ActivoId::from_uuid(c.id), c.nombre, c.costo))
        .collect();
        Ok(TableroMantenimiento {
            preventivas_programadas: entero(r.programadas, "programadas")?,
            preventivas_cumplidas: entero(r.cumplidas, "cumplidas")?,
            ordenes_vencidas: entero(r.vencidas, "vencidas")?,
            costo_por_activo: costos,
        })
    }
}
