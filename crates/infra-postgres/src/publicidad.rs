//! Publicidad de la plataforma (plan 08, fases 3 a 6; ADR 0015).
//!
//! - [`RepositorioPublicidadPlataforma`] sobre la transacción de plataforma: campañas,
//!   creativos, tarifas, política, liquidaciones y redención (tablas `solo_plataforma`); los
//!   datos de las PH (contratos para el conflicto de interés, contactos, reportes, outbox) por
//!   funciones `SECURITY DEFINER`.
//! - [`RepositorioPublicidad`] sobre la transacción del tenant: piezas y entregas por
//!   `piezas_publicidad`, `pieza_para_entrega` (bloquea la campaña) y
//!   `aplicar_entrega_publicidad`; envíos masivos, reportes y pantalla con RLS.
//! - [`DirectorioPublicidadPg`] lista las copropiedades con el módulo (worker).

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::{
        DirectorioPublicidad, RepositorioPublicidad, RepositorioPublicidadPlataforma,
        directorio::Anunciante,
        publicidad::{
            Campana, ConsumoPeriodo, Creativo, CreativoReportado, DestinatarioMasivo, Lead,
            Liquidacion, MetricaCampana, PantallaPublicidad, PiezaPublicidad, PoliticaPublicidad,
            ReportePublicidad, TarifaPublicidad,
        },
    },
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    AnuncianteId, CampanaId, CanalPublicidad, CodigoCategoria, CodigoCupon, CodigoDane, CreativoId,
    CuentaId, EstadoCampana, EstadoCreativo, EstadoLiquidacion, FormatoCreativo, LiquidacionId,
    MedioAutorizacion, ModeloCobro, Nit, ObjetivoCampana, OrganizacionId, PersonaId,
    PoliticaPublicidadId, RolOrganizacion, Segmentacion, SolicitudProveedorId, Telefono, TenantId,
    TipoCopropiedad,
    publicidad::{
        ContenidoCreativo, Conteo, Cupon, FRECUENCIA_MASIVA_DIAS, PiezaCandidata, Presupuesto,
    },
};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    TransaccionPg, TransaccionPlataformaPg, directorio::FilaAnunciante,
    plataforma::conflicto_si_unico,
};

fn u32_de(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

fn i32_de(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

// ---------------------------------------------------------------------------------------------
// Filas
// ---------------------------------------------------------------------------------------------

struct FilaCampana {
    id: Uuid,
    anunciante_id: Uuid,
    nombre: String,
    objetivo: String,
    categoria: String,
    inicio: DateTime<Utc>,
    fin: DateTime<Utc>,
    modelo: String,
    presupuesto_total: Decimal,
    presupuesto_diario: Option<Decimal>,
    tarifa: Option<Decimal>,
    consumido: Decimal,
    seg_radio_km: Option<i16>,
    seg_municipios: Vec<String>,
    seg_tipos: Vec<String>,
    seg_bienes_min: Option<i32>,
    seg_bienes_max: Option<i32>,
    seg_estratos: Vec<i16>,
    copropiedades: Vec<Uuid>,
    conflicto_interes: bool,
    estado: String,
    motivo_estado: Option<String>,
    revisada_por: Option<Uuid>,
    revisada_en: Option<DateTime<Utc>>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
    actualizada_en: DateTime<Utc>,
}

impl TryFrom<FilaCampana> for Campana {
    type Error = AppError;

    fn try_from(f: FilaCampana) -> AppResult<Self> {
        let segmentacion = Segmentacion {
            copropiedades: f
                .copropiedades
                .into_iter()
                .map(TenantId::from_uuid)
                .collect(),
            radio_km: f.seg_radio_km.and_then(|r| u16::try_from(r).ok()),
            municipios: f
                .seg_municipios
                .iter()
                .map(|m| CodigoDane::new(m))
                .collect::<Result<_, _>>()?,
            tipos: f
                .seg_tipos
                .iter()
                .map(|t| TipoCopropiedad::try_from(t.as_str()))
                .collect::<Result<_, _>>()?,
            bienes_min: f.seg_bienes_min.and_then(|v| u32::try_from(v).ok()),
            bienes_max: f.seg_bienes_max.and_then(|v| u32::try_from(v).ok()),
            estratos: f
                .seg_estratos
                .iter()
                .filter_map(|e| u8::try_from(*e).ok())
                .collect(),
        };
        Ok(Self {
            id: CampanaId::from_uuid(f.id),
            anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
            nombre: f.nombre,
            objetivo: ObjetivoCampana::try_from(f.objetivo.as_str())?,
            categoria: CodigoCategoria::new(&f.categoria)?,
            inicio: f.inicio,
            fin: f.fin,
            modelo: ModeloCobro::try_from(f.modelo.as_str())?,
            presupuesto: Presupuesto::new(f.presupuesto_total, f.presupuesto_diario)?,
            tarifa: f.tarifa,
            consumido: f.consumido,
            segmentacion,
            conflicto_interes: f.conflicto_interes,
            estado: EstadoCampana::try_from(f.estado.as_str())?,
            motivo_estado: f.motivo_estado,
            revisada_por: f.revisada_por.map(CuentaId::from_uuid),
            revisada_en: f.revisada_en,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
            actualizada_en: f.actualizada_en,
        })
    }
}

struct FilaCreativo {
    id: Uuid,
    campana_id: Uuid,
    anunciante_id: Uuid,
    formato: String,
    titulo: String,
    texto: String,
    boton: Option<String>,
    enlace: Option<String>,
    condiciones: Option<String>,
    es_promocion: bool,
    cupon_codigo: Option<String>,
    cupon_limite: Option<i32>,
    cupon_usos: i32,
    plantilla_whatsapp: Option<String>,
    estado: String,
    motivo_estado: Option<String>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

fn cupon(codigo: Option<&str>, limite: Option<i32>) -> AppResult<Option<Cupon>> {
    codigo
        .map(|c| {
            Ok(Cupon {
                codigo: CodigoCupon::new(c)?,
                limite_usos: limite.and_then(|l| u32::try_from(l).ok()),
            })
        })
        .transpose()
}

impl TryFrom<FilaCreativo> for Creativo {
    type Error = AppError;

    fn try_from(f: FilaCreativo) -> AppResult<Self> {
        Ok(Self {
            id: CreativoId::from_uuid(f.id),
            campana_id: CampanaId::from_uuid(f.campana_id),
            anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
            contenido: ContenidoCreativo {
                formato: FormatoCreativo::try_from(f.formato.as_str())?,
                titulo: f.titulo,
                texto: f.texto,
                boton: f.boton,
                enlace: f.enlace,
                condiciones: f.condiciones,
                es_promocion: f.es_promocion,
                cupon: cupon(f.cupon_codigo.as_deref(), f.cupon_limite)?,
                plantilla_whatsapp: f.plantilla_whatsapp,
            },
            cupon_usos: u32::try_from(f.cupon_usos).unwrap_or(0),
            estado: EstadoCreativo::try_from(f.estado.as_str())?,
            motivo_estado: f.motivo_estado,
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

struct FilaLiquidacion {
    id: Uuid,
    anunciante_id: Uuid,
    periodo: NaiveDate,
    variable: Decimal,
    fijos: Decimal,
    leads: i32,
    valor_leads: Decimal,
    valor: Decimal,
    costo_whatsapp: Decimal,
    estado: String,
    calculada_en: DateTime<Utc>,
    actualizada_en: DateTime<Utc>,
}

impl TryFrom<FilaLiquidacion> for Liquidacion {
    type Error = AppError;

    fn try_from(f: FilaLiquidacion) -> AppResult<Self> {
        Ok(Self {
            id: LiquidacionId::from_uuid(f.id),
            anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
            periodo: f.periodo,
            variable: f.variable,
            fijos: f.fijos,
            leads: u32::try_from(f.leads).unwrap_or(0),
            valor_leads: f.valor_leads,
            valor: f.valor,
            costo_whatsapp: f.costo_whatsapp,
            estado: EstadoLiquidacion::try_from(f.estado.as_str())?,
            calculada_en: f.calculada_en,
            actualizada_en: f.actualizada_en,
        })
    }
}

struct FilaPolitica {
    id: Uuid,
    version: i32,
    texto: String,
    vigente_desde: DateTime<Utc>,
    publicada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl From<FilaPolitica> for PoliticaPublicidad {
    fn from(f: FilaPolitica) -> Self {
        Self {
            id: PoliticaPublicidadId::from_uuid(f.id),
            version: u32::try_from(f.version).unwrap_or(0),
            texto: f.texto,
            vigente_desde: f.vigente_desde,
            publicada_por: f.publicada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        }
    }
}

fn textos<T: Copy>(v: &[T], f: fn(T) -> &'static str) -> Vec<String> {
    v.iter().map(|x| f(*x).to_owned()).collect()
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

impl TransaccionPlataformaPg {
    async fn guardar_copropiedades(&mut self, c: &Campana) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM campana_copropiedades WHERE campana_id = $1",
            c.id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = c
            .segmentacion
            .copropiedades
            .iter()
            .map(|t| t.as_uuid())
            .collect();
        sqlx::query!(
            "INSERT INTO campana_copropiedades (campana_id, tenant_id)
             SELECT $1, unnest($2::uuid[])",
            c.id.as_uuid(),
            &ids,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|d| d.is_foreign_key_violation())
            {
                AppError::Validacion("alguna copropiedad de la segmentación no existe".into())
            } else {
                AppError::infra(e)
            }
        })?;
        Ok(())
    }
}

impl RepositorioPublicidadPlataforma for TransaccionPlataformaPg {
    async fn organizacion_por_nit(&mut self, nit: &Nit) -> AppResult<Option<OrganizacionId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM organizaciones WHERE nit = $1",
            nit.to_string()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(OrganizacionId::from_uuid))
    }

    async fn anunciantes_de_cuenta(
        &mut self,
        cuenta: CuentaId,
    ) -> AppResult<Vec<(Anunciante, RolOrganizacion)>> {
        let filas = sqlx::query!(
            "SELECT a.id, a.organizacion_id, a.nombre_comercial, a.direccion, a.municipio_dane,
                    a.latitud, a.longitud, a.estado_verificacion, a.motivo_estado, a.creado_por,
                    a.creado_en, a.actualizado_en, m.rol
             FROM anunciantes a
             JOIN organizaciones_miembros m ON m.organizacion_id = a.organizacion_id
             WHERE m.cuenta_id = $1 ORDER BY a.nombre_comercial, a.id",
            cuenta.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                let rol = RolOrganizacion::try_from(f.rol.as_str())?;
                let a = Anunciante::try_from(FilaAnunciante {
                    id: f.id,
                    organizacion_id: f.organizacion_id,
                    nombre_comercial: f.nombre_comercial,
                    direccion: f.direccion,
                    municipio_dane: f.municipio_dane,
                    latitud: f.latitud,
                    longitud: f.longitud,
                    estado_verificacion: f.estado_verificacion,
                    motivo_estado: f.motivo_estado,
                    creado_por: f.creado_por,
                    creado_en: f.creado_en,
                    actualizado_en: f.actualizado_en,
                })?;
                Ok((a, rol))
            })
            .collect()
    }

    async fn insertar_campana(&mut self, c: &Campana) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO campanas (id, anunciante_id, nombre, objetivo, categoria, inicio, fin,
                 modelo, presupuesto_total, presupuesto_diario, tarifa, consumido, seg_radio_km,
                 seg_municipios, seg_tipos, seg_bienes_min, seg_bienes_max, seg_estratos,
                 conflicto_interes, estado, motivo_estado, revisada_por, revisada_en, creada_por,
                 creada_en, actualizada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21, $22, $23, $24, $25, $26)",
            c.id.as_uuid(),
            c.anunciante_id.as_uuid(),
            c.nombre,
            c.objetivo.as_str(),
            c.categoria.as_str(),
            c.inicio,
            c.fin,
            c.modelo.as_str(),
            c.presupuesto.total,
            c.presupuesto.diario,
            c.tarifa,
            c.consumido,
            c.segmentacion
                .radio_km
                .map(|r| i16::try_from(r).unwrap_or(i16::MAX)),
            &c.segmentacion
                .municipios
                .iter()
                .map(|m| m.as_str().to_owned())
                .collect::<Vec<_>>(),
            &textos(&c.segmentacion.tipos, TipoCopropiedad::as_str),
            c.segmentacion.bienes_min.map(i32_de),
            c.segmentacion.bienes_max.map(i32_de),
            &c.segmentacion
                .estratos
                .iter()
                .map(|e| i16::from(*e))
                .collect::<Vec<_>>(),
            c.conflicto_interes,
            c.estado.as_str(),
            c.motivo_estado,
            c.revisada_por.map(CuentaId::as_uuid),
            c.revisada_en,
            c.creada_por.map(CuentaId::as_uuid),
            c.creada_en,
            c.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|d| d.is_check_violation())
            {
                AppError::Validacion("la categoría de la campaña está restringida".into())
            } else {
                AppError::infra(e)
            }
        })?;
        self.guardar_copropiedades(c).await
    }

    async fn actualizar_campana(&mut self, c: &Campana) -> AppResult<()> {
        sqlx::query!(
            "UPDATE campanas SET nombre = $2, objetivo = $3, categoria = $4, inicio = $5,
                 fin = $6, modelo = $7, presupuesto_total = $8, presupuesto_diario = $9,
                 tarifa = $10, seg_radio_km = $11, seg_municipios = $12, seg_tipos = $13,
                 seg_bienes_min = $14, seg_bienes_max = $15, seg_estratos = $16,
                 conflicto_interes = $17, estado = $18, motivo_estado = $19, revisada_por = $20,
                 revisada_en = $21, actualizada_en = $22
             WHERE id = $1",
            c.id.as_uuid(),
            c.nombre,
            c.objetivo.as_str(),
            c.categoria.as_str(),
            c.inicio,
            c.fin,
            c.modelo.as_str(),
            c.presupuesto.total,
            c.presupuesto.diario,
            c.tarifa,
            c.segmentacion
                .radio_km
                .map(|r| i16::try_from(r).unwrap_or(i16::MAX)),
            &c.segmentacion
                .municipios
                .iter()
                .map(|m| m.as_str().to_owned())
                .collect::<Vec<_>>(),
            &textos(&c.segmentacion.tipos, TipoCopropiedad::as_str),
            c.segmentacion.bienes_min.map(i32_de),
            c.segmentacion.bienes_max.map(i32_de),
            &c.segmentacion
                .estratos
                .iter()
                .map(|e| i16::from(*e))
                .collect::<Vec<_>>(),
            c.conflicto_interes,
            c.estado.as_str(),
            c.motivo_estado,
            c.revisada_por.map(CuentaId::as_uuid),
            c.revisada_en,
            c.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.guardar_copropiedades(c).await
    }

    async fn campana(&mut self, id: CampanaId, bloquear: bool) -> AppResult<Option<Campana>> {
        if bloquear {
            sqlx::query!(
                "SELECT 1 AS x FROM campanas WHERE id = $1 FOR UPDATE",
                id.as_uuid()
            )
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        sqlx::query_as!(
            FilaCampana,
            r#"SELECT c.id, c.anunciante_id, c.nombre, c.objetivo, c.categoria, c.inicio, c.fin,
                      c.modelo, c.presupuesto_total, c.presupuesto_diario, c.tarifa, c.consumido,
                      c.seg_radio_km, c.seg_municipios, c.seg_tipos, c.seg_bienes_min,
                      c.seg_bienes_max, c.seg_estratos,
                      ARRAY(SELECT x.tenant_id FROM campana_copropiedades x
                            WHERE x.campana_id = c.id ORDER BY 1) AS "copropiedades!",
                      c.conflicto_interes, c.estado, c.motivo_estado, c.revisada_por,
                      c.revisada_en, c.creada_por, c.creada_en, c.actualizada_en
               FROM campanas c WHERE c.id = $1"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Campana::try_from)
        .transpose()
    }

    async fn campanas_de_anunciante(
        &mut self,
        anunciante: AnuncianteId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Campana>> {
        let filas = sqlx::query_as!(
            FilaCampana,
            r#"SELECT c.id, c.anunciante_id, c.nombre, c.objetivo, c.categoria, c.inicio, c.fin,
                      c.modelo, c.presupuesto_total, c.presupuesto_diario, c.tarifa, c.consumido,
                      c.seg_radio_km, c.seg_municipios, c.seg_tipos, c.seg_bienes_min,
                      c.seg_bienes_max, c.seg_estratos,
                      ARRAY(SELECT x.tenant_id FROM campana_copropiedades x
                            WHERE x.campana_id = c.id ORDER BY 1) AS "copropiedades!",
                      c.conflicto_interes, c.estado, c.motivo_estado, c.revisada_por,
                      c.revisada_en, c.creada_por, c.creada_en, c.actualizada_en
               FROM campanas c
               WHERE c.anunciante_id = $1 AND ($2::uuid IS NULL OR c.id > $2)
               ORDER BY c.id LIMIT $3"#,
            anunciante.as_uuid(),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Campana::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |c| c.id.as_uuid()))
    }

    async fn campanas_en_revision(&mut self, p: SolicitudPagina) -> AppResult<Pagina<Campana>> {
        let filas = sqlx::query_as!(
            FilaCampana,
            r#"SELECT c.id, c.anunciante_id, c.nombre, c.objetivo, c.categoria, c.inicio, c.fin,
                      c.modelo, c.presupuesto_total, c.presupuesto_diario, c.tarifa, c.consumido,
                      c.seg_radio_km, c.seg_municipios, c.seg_tipos, c.seg_bienes_min,
                      c.seg_bienes_max, c.seg_estratos,
                      ARRAY(SELECT x.tenant_id FROM campana_copropiedades x
                            WHERE x.campana_id = c.id ORDER BY 1) AS "copropiedades!",
                      c.conflicto_interes, c.estado, c.motivo_estado, c.revisada_por,
                      c.revisada_en, c.creada_por, c.creada_en, c.actualizada_en
               FROM campanas c
               WHERE c.estado = 'en_revision' AND ($1::uuid IS NULL OR c.id > $1)
               ORDER BY c.id LIMIT $2"#,
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Campana::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |c| c.id.as_uuid()))
    }

    async fn finalizar_vencidas(&mut self, ahora: DateTime<Utc>) -> AppResult<u64> {
        Ok(sqlx::query!(
            "UPDATE campanas SET estado = 'finalizada', actualizada_en = $1
             WHERE estado IN ('aprobada', 'activa', 'pausada') AND fin <= $1",
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected())
    }

    async fn insertar_creativo(&mut self, c: &Creativo) -> AppResult<()> {
        let k = &c.contenido;
        let cup = k.cupon.as_ref();
        sqlx::query!(
            "INSERT INTO creativos (id, campana_id, anunciante_id, formato, titulo, texto, boton,
                 enlace, condiciones, es_promocion, cupon_codigo, cupon_limite, cupon_usos,
                 plantilla_whatsapp, estado, motivo_estado, creado_en, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18)",
            c.id.as_uuid(),
            c.campana_id.as_uuid(),
            c.anunciante_id.as_uuid(),
            k.formato.as_str(),
            k.titulo,
            k.texto,
            k.boton,
            k.enlace,
            k.condiciones,
            k.es_promocion,
            cup.map(|x| x.codigo.as_str()),
            cup.and_then(|x| x.limite_usos).map(i32_de),
            i32_de(c.cupon_usos),
            k.plantilla_whatsapp,
            c.estado.as_str(),
            c.motivo_estado,
            c.creado_en,
            c.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya tiene otra oferta con ese código de cupón"))?;
        Ok(())
    }

    async fn actualizar_creativo(&mut self, c: &Creativo) -> AppResult<()> {
        let k = &c.contenido;
        let cup = k.cupon.as_ref();
        sqlx::query!(
            "UPDATE creativos SET formato = $2, titulo = $3, texto = $4, boton = $5, enlace = $6,
                 condiciones = $7, es_promocion = $8, cupon_codigo = $9, cupon_limite = $10,
                 plantilla_whatsapp = $11, estado = $12, motivo_estado = $13,
                 actualizado_en = $14
             WHERE id = $1",
            c.id.as_uuid(),
            k.formato.as_str(),
            k.titulo,
            k.texto,
            k.boton,
            k.enlace,
            k.condiciones,
            k.es_promocion,
            cup.map(|x| x.codigo.as_str()),
            cup.and_then(|x| x.limite_usos).map(i32_de),
            k.plantilla_whatsapp,
            c.estado.as_str(),
            c.motivo_estado,
            c.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya tiene otra oferta con ese código de cupón"))?;
        Ok(())
    }

    async fn creativo(&mut self, id: CreativoId) -> AppResult<Option<Creativo>> {
        sqlx::query_as!(
            FilaCreativo,
            "SELECT id, campana_id, anunciante_id, formato, titulo, texto, boton, enlace,
                    condiciones, es_promocion, cupon_codigo, cupon_limite, cupon_usos,
                    plantilla_whatsapp, estado, motivo_estado, creado_en, actualizado_en
             FROM creativos WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Creativo::try_from)
        .transpose()
    }

    async fn creativos_de_campana(&mut self, campana: CampanaId) -> AppResult<Vec<Creativo>> {
        sqlx::query_as!(
            FilaCreativo,
            "SELECT id, campana_id, anunciante_id, formato, titulo, texto, boton, enlace,
                    condiciones, es_promocion, cupon_codigo, cupon_limite, cupon_usos,
                    plantilla_whatsapp, estado, motivo_estado, creado_en, actualizado_en
             FROM creativos WHERE campana_id = $1 ORDER BY id",
            campana.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Creativo::try_from)
        .collect()
    }

    async fn creativos_pendientes(&mut self, p: SolicitudPagina) -> AppResult<Pagina<Creativo>> {
        let filas = sqlx::query_as!(
            FilaCreativo,
            "SELECT id, campana_id, anunciante_id, formato, titulo, texto, boton, enlace,
                    condiciones, es_promocion, cupon_codigo, cupon_limite, cupon_usos,
                    plantilla_whatsapp, estado, motivo_estado, creado_en, actualizado_en
             FROM creativos
             WHERE estado = 'pendiente' AND ($1::uuid IS NULL OR id > $1)
             ORDER BY id LIMIT $2",
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Creativo::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |c| c.id.as_uuid()))
    }

    async fn creativo_por_cupon(
        &mut self,
        anunciante: AnuncianteId,
        codigo: &CodigoCupon,
    ) -> AppResult<Option<Creativo>> {
        let id = sqlx::query_scalar!(
            "SELECT cr.id FROM creativos cr JOIN campanas c ON c.id = cr.campana_id
             WHERE cr.anunciante_id = $1 AND cr.cupon_codigo = $2
             FOR UPDATE OF c, cr",
            anunciante.as_uuid(),
            codigo.as_str(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match id {
            Some(id) => self.creativo(CreativoId::from_uuid(id)).await,
            None => Ok(None),
        }
    }

    async fn registrar_redencion(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
        costo: Decimal,
        finalizar: bool,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE creativos SET cupon_usos = cupon_usos + 1, actualizado_en = $2 WHERE id = $1",
            creativo.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "SELECT aplicar_entrega_publicidad($1, 'app', $2, 0, 0, 1, $3, $4)",
            creativo.as_uuid(),
            ahora,
            costo,
            finalizar,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn conflicto_interes(&mut self, campana: CampanaId) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT conflicto_interes_campana($1) AS "c!""#,
            campana.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn alcance(&mut self, campana: CampanaId) -> AppResult<u64> {
        let n = sqlx::query_scalar!(r#"SELECT alcance_campana($1) AS "n!""#, campana.as_uuid())
            .fetch_one(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn tarifa_vigente(
        &mut self,
        modelo: ModeloCobro,
        dia: NaiveDate,
    ) -> AppResult<Option<TarifaPublicidad>> {
        sqlx::query!(
            "SELECT modelo, valor, vigente_desde FROM tarifas_publicidad
             WHERE modelo = $1 AND vigente_desde <= $2
             ORDER BY vigente_desde DESC LIMIT 1",
            modelo.as_str(),
            dia,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(TarifaPublicidad {
                modelo: ModeloCobro::try_from(f.modelo.as_str())?,
                valor: f.valor,
                vigente_desde: f.vigente_desde,
            })
        })
        .transpose()
    }

    async fn tarifas(&mut self) -> AppResult<Vec<TarifaPublicidad>> {
        sqlx::query!(
            "SELECT modelo, valor, vigente_desde FROM tarifas_publicidad
             ORDER BY modelo, vigente_desde"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(TarifaPublicidad {
                modelo: ModeloCobro::try_from(f.modelo.as_str())?,
                valor: f.valor,
                vigente_desde: f.vigente_desde,
            })
        })
        .collect()
    }

    async fn insertar_tarifa(
        &mut self,
        t: &TarifaPublicidad,
        creada_por: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO tarifas_publicidad (id, modelo, valor, vigente_desde, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            Uuid::now_v7(),
            t.modelo.as_str(),
            t.valor,
            t.vigente_desde,
            creada_por.map(CuentaId::as_uuid),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya hay una tarifa de ese modelo desde esa fecha"))?;
        Ok(())
    }

    async fn metricas_campana(&mut self, campana: CampanaId) -> AppResult<Vec<MetricaCampana>> {
        sqlx::query!(
            r#"SELECT dia, canal, sum(impresiones)::bigint AS "impresiones!",
                      sum(clics)::bigint AS "clics!", sum(redenciones)::bigint AS "redenciones!",
                      sum(costo) AS "costo!"
               FROM entregas_publicidad WHERE campana_id = $1
               GROUP BY dia, canal ORDER BY dia, canal"#,
            campana.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(MetricaCampana {
                dia: f.dia,
                canal: CanalPublicidad::try_from(f.canal.as_str())?,
                conteo: Conteo {
                    impresiones: u32_de(f.impresiones),
                    clics: u32_de(f.clics),
                    redenciones: u32_de(f.redenciones),
                    leads: 0,
                },
                costo: f.costo,
            })
        })
        .collect()
    }

    async fn leads(
        &mut self,
        anunciante: AnuncianteId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Lead>> {
        let filas = sqlx::query!(
            r#"SELECT solicitud_id AS "solicitud_id!", creado_en AS "creado_en!", categoria,
                      nombre AS "nombre!", telefono
               FROM leads_anunciante($1, $2, $3::int)"#,
            anunciante.as_uuid(),
            p.despues(),
            i32::try_from(p.filas_a_consultar()).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(|f| {
                Ok(Lead {
                    solicitud_id: SolicitudProveedorId::from_uuid(f.solicitud_id),
                    creado_en: f.creado_en,
                    categoria: f
                        .categoria
                        .as_deref()
                        .map(CodigoCategoria::new)
                        .transpose()?,
                    nombre: f.nombre,
                    telefono: f.telefono.and_then(|t| Telefono::new(&t).ok()),
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |l| l.solicitud_id.as_uuid()))
    }

    async fn consumo_periodo(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<ConsumoPeriodo>> {
        let filas = sqlx::query!(
            r#"WITH v AS (
                   SELECT c.anunciante_id, sum(e.costo) AS variable
                   FROM entregas_publicidad e JOIN campanas c ON c.id = e.campana_id
                   WHERE e.dia >= $1 AND e.dia < $2 GROUP BY 1
               ), f AS (
                   SELECT c.anunciante_id, sum(c.tarifa) AS fijos
                   FROM campanas c
                   WHERE c.modelo IN ('tarifa_fija', 'directorio_destacado')
                     AND c.tarifa IS NOT NULL
                     AND c.estado IN ('activa', 'pausada', 'finalizada')
                     AND c.inicio < $2::date AND c.fin > $1::date
                   GROUP BY 1
               ), d AS (
                   SELECT anunciante_id, leads, mensajes_whatsapp
                   FROM consumo_directo_anunciantes($1::date, $2::date)
               )
               SELECT a.id AS "anunciante_id!", coalesce(v.variable, 0) AS "variable!",
                      coalesce(f.fijos, 0) AS "fijos!", coalesce(d.leads, 0) AS "leads!",
                      coalesce(d.mensajes_whatsapp, 0) AS "mensajes!"
               FROM anunciantes a
               LEFT JOIN v ON v.anunciante_id = a.id
               LEFT JOIN f ON f.anunciante_id = a.id
               LEFT JOIN d ON d.anunciante_id = a.id
               WHERE v.anunciante_id IS NOT NULL OR f.anunciante_id IS NOT NULL
                  OR d.anunciante_id IS NOT NULL
               ORDER BY a.id"#,
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(filas
            .into_iter()
            .map(|f| ConsumoPeriodo {
                anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
                variable: f.variable,
                fijos: f.fijos,
                leads: u32_de(f.leads),
                mensajes_whatsapp: u32_de(f.mensajes),
            })
            .collect())
    }

    async fn tarifa_whatsapp_marketing(&mut self, dia: NaiveDate) -> AppResult<Option<Decimal>> {
        sqlx::query_scalar!(
            "SELECT valor FROM tarifas_whatsapp
             WHERE pais = 'CO' AND categoria = 'marketing' AND moneda = 'COP'
               AND vigente_desde <= $1
             ORDER BY vigente_desde DESC LIMIT 1",
            dia,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn liquidacion(
        &mut self,
        anunciante: AnuncianteId,
        periodo: NaiveDate,
    ) -> AppResult<Option<Liquidacion>> {
        sqlx::query_as!(
            FilaLiquidacion,
            "SELECT id, anunciante_id, periodo, variable, fijos, leads, valor_leads, valor,
                    costo_whatsapp, estado, calculada_en, actualizada_en
             FROM liquidaciones WHERE anunciante_id = $1 AND periodo = $2 FOR UPDATE",
            anunciante.as_uuid(),
            periodo,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Liquidacion::try_from)
        .transpose()
    }

    async fn guardar_liquidacion(&mut self, l: &Liquidacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO liquidaciones (id, anunciante_id, periodo, variable, fijos, leads,
                 valor_leads, valor, costo_whatsapp, estado, calculada_en, actualizada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (anunciante_id, periodo) DO UPDATE
                 SET variable = EXCLUDED.variable, fijos = EXCLUDED.fijos,
                     leads = EXCLUDED.leads, valor_leads = EXCLUDED.valor_leads,
                     valor = EXCLUDED.valor, costo_whatsapp = EXCLUDED.costo_whatsapp,
                     estado = EXCLUDED.estado, actualizada_en = EXCLUDED.actualizada_en",
            l.id.as_uuid(),
            l.anunciante_id.as_uuid(),
            l.periodo,
            l.variable,
            l.fijos,
            i32_de(l.leads),
            l.valor_leads,
            l.valor,
            l.costo_whatsapp,
            l.estado.as_str(),
            l.calculada_en,
            l.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn liquidaciones(&mut self, periodo: NaiveDate) -> AppResult<Vec<Liquidacion>> {
        sqlx::query_as!(
            FilaLiquidacion,
            "SELECT id, anunciante_id, periodo, variable, fijos, leads, valor_leads, valor,
                    costo_whatsapp, estado, calculada_en, actualizada_en
             FROM liquidaciones WHERE periodo = $1 ORDER BY anunciante_id",
            periodo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Liquidacion::try_from)
        .collect()
    }

    async fn reportes_pendientes(
        &mut self,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<CreativoReportado>> {
        let filas = sqlx::query!(
            r#"SELECT creativo_id AS "creativo_id!", campana_id AS "campana_id!",
                      anunciante_id AS "anunciante_id!", nombre_comercial AS "nombre_comercial!",
                      titulo AS "titulo!", reportes AS "reportes!", motivos AS "motivos!",
                      primero_en AS "primero_en!"
               FROM reportes_publicidad_pendientes($1, $2::int)"#,
            p.despues(),
            i32::try_from(p.filas_a_consultar()).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(|f| CreativoReportado {
                creativo_id: CreativoId::from_uuid(f.creativo_id),
                campana_id: CampanaId::from_uuid(f.campana_id),
                anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
                nombre_comercial: f.nombre_comercial,
                titulo: f.titulo,
                reportes: u32_de(f.reportes),
                motivos: f.motivos,
                primero_en: f.primero_en,
            })
            .collect();
        Ok(Pagina::desde_filas(v, p, |r| r.creativo_id.as_uuid()))
    }

    async fn atender_reportes(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT atender_reportes_publicidad($1, $2) AS "n!""#,
            creativo.as_uuid(),
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn politica_publicidad_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<PoliticaPublicidad>> {
        Ok(sqlx::query_as!(
            FilaPolitica,
            "SELECT id, version, texto, vigente_desde, publicada_por, creada_en
             FROM politicas_publicidad WHERE vigente_desde <= $1
             ORDER BY version DESC LIMIT 1",
            ahora,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Into::into))
    }

    async fn ultima_version_politica(&mut self) -> AppResult<u32> {
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM politicas_publicidad"#
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u32::try_from(v).unwrap_or(0))
    }

    async fn insertar_politica_publicidad(&mut self, p: &PoliticaPublicidad) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO politicas_publicidad (id, version, texto, vigente_desde, publicada_por,
                                               creada_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            p.id.as_uuid(),
            i32_de(p.version),
            p.texto,
            p.vigente_desde,
            p.publicada_por.map(CuentaId::as_uuid),
            p.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "otra versión se publicó al mismo tiempo"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Copropiedad
// ---------------------------------------------------------------------------------------------

/// Columnas que deciden si una pieza se puede entregar, comunes a `piezas_publicidad` y
/// `pieza_para_entrega` (campos nombrados: varios son `Decimal` y se confundirían por posición).
struct FilaCandidata<'a> {
    creativo: Uuid,
    campana: Uuid,
    verificado: bool,
    estado_creativo: &'a str,
    estado_campana: &'a str,
    inicio: DateTime<Utc>,
    fin: DateTime<Utc>,
    modelo: &'a str,
    tarifa: Option<Decimal>,
    total: Decimal,
    diario: Option<Decimal>,
    consumido: Decimal,
    consumido_hoy: Decimal,
    impresiones_hoy: i64,
}

impl TryFrom<FilaCandidata<'_>> for PiezaCandidata {
    type Error = AppError;

    fn try_from(f: FilaCandidata<'_>) -> AppResult<Self> {
        Ok(Self {
            creativo: CreativoId::from_uuid(f.creativo),
            campana: CampanaId::from_uuid(f.campana),
            anunciante_verificado: f.verificado,
            creativo_aprobado: EstadoCreativo::try_from(f.estado_creativo)?
                == EstadoCreativo::Aprobado,
            estado_campana: EstadoCampana::try_from(f.estado_campana)?,
            inicio: f.inicio,
            fin: f.fin,
            modelo: ModeloCobro::try_from(f.modelo)?,
            // Una campaña activa siempre tiene tarifa (CHECK); sin ella no se cobra.
            tarifa: f.tarifa.unwrap_or(Decimal::ZERO),
            presupuesto: Presupuesto::new(f.total, f.diario)?,
            consumido_total: f.consumido,
            consumido_hoy: f.consumido_hoy,
            impresiones_hoy: u32_de(f.impresiones_hoy),
        })
    }
}

impl RepositorioPublicidad for TransaccionPg {
    async fn piezas(
        &mut self,
        formatos: &[FormatoCreativo],
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<PiezaPublicidad>> {
        let formatos = textos(formatos, FormatoCreativo::as_str);
        let filas = sqlx::query!(
            r#"SELECT creativo_id AS "creativo_id!", campana_id AS "campana_id!",
                      anunciante_id AS "anunciante_id!", nombre_comercial AS "nombre_comercial!",
                      anunciante_verificado AS "anunciante_verificado!", formato AS "formato!",
                      titulo AS "titulo!", texto AS "texto!", boton, enlace, condiciones,
                      es_promocion AS "es_promocion!", cupon_codigo, plantilla_whatsapp,
                      estado_creativo AS "estado_creativo!", estado_campana AS "estado_campana!",
                      inicio AS "inicio!", fin AS "fin!", modelo AS "modelo!", tarifa,
                      presupuesto_total AS "presupuesto_total!", presupuesto_diario,
                      consumido AS "consumido!", consumido_hoy AS "consumido_hoy!",
                      impresiones_hoy AS "impresiones_hoy!"
               FROM piezas_publicidad($1, $2)"#,
            &formatos,
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                Ok(PiezaPublicidad {
                    candidata: FilaCandidata {
                        creativo: f.creativo_id,
                        campana: f.campana_id,
                        verificado: f.anunciante_verificado,
                        estado_creativo: &f.estado_creativo,
                        estado_campana: &f.estado_campana,
                        inicio: f.inicio,
                        fin: f.fin,
                        modelo: &f.modelo,
                        tarifa: f.tarifa,
                        total: f.presupuesto_total,
                        diario: f.presupuesto_diario,
                        consumido: f.consumido,
                        consumido_hoy: f.consumido_hoy,
                        impresiones_hoy: f.impresiones_hoy,
                    }
                    .try_into()?,
                    anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
                    nombre_comercial: f.nombre_comercial,
                    contenido: ContenidoCreativo {
                        formato: FormatoCreativo::try_from(f.formato.as_str())?,
                        titulo: f.titulo,
                        texto: f.texto,
                        boton: f.boton,
                        enlace: f.enlace,
                        condiciones: f.condiciones,
                        es_promocion: f.es_promocion,
                        cupon: cupon(f.cupon_codigo.as_deref(), None)?,
                        plantilla_whatsapp: f.plantilla_whatsapp,
                    },
                })
            })
            .collect()
    }

    async fn pieza_para_entrega(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<(PiezaCandidata, FormatoCreativo)>> {
        let f = sqlx::query!(
            r#"SELECT creativo_id AS "creativo_id!", campana_id AS "campana_id!",
                      anunciante_verificado AS "anunciante_verificado!", formato AS "formato!",
                      estado_creativo AS "estado_creativo!", estado_campana AS "estado_campana!",
                      inicio AS "inicio!", fin AS "fin!", modelo AS "modelo!", tarifa,
                      presupuesto_total AS "presupuesto_total!", presupuesto_diario,
                      consumido AS "consumido!", consumido_hoy AS "consumido_hoy!"
               FROM pieza_para_entrega($1, $2)"#,
            creativo.as_uuid(),
            ahora,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        f.map(|f| {
            Ok((
                PiezaCandidata::try_from(FilaCandidata {
                    creativo: f.creativo_id,
                    campana: f.campana_id,
                    verificado: f.anunciante_verificado,
                    estado_creativo: &f.estado_creativo,
                    estado_campana: &f.estado_campana,
                    inicio: f.inicio,
                    fin: f.fin,
                    modelo: &f.modelo,
                    tarifa: f.tarifa,
                    total: f.presupuesto_total,
                    diario: f.presupuesto_diario,
                    consumido: f.consumido,
                    consumido_hoy: f.consumido_hoy,
                    // Solo ordena la rotación al seleccionar piezas; la entrega no lo usa.
                    impresiones_hoy: 0,
                })?,
                FormatoCreativo::try_from(f.formato.as_str())?,
            ))
        })
        .transpose()
    }

    async fn aplicar_entrega(
        &mut self,
        creativo: CreativoId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
        conteo: Conteo,
        costo: Decimal,
        finalizar: bool,
    ) -> AppResult<()> {
        sqlx::query!(
            "SELECT aplicar_entrega_publicidad($1, $2, $3, $4, $5, $6, $7, $8)",
            creativo.as_uuid(),
            canal.as_str(),
            ahora,
            i32_de(conteo.impresiones),
            i32_de(conteo.clics),
            i32_de(conteo.redenciones),
            costo,
            finalizar,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn politica_publicidad_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<PoliticaPublicidad>> {
        Ok(sqlx::query_as!(
            FilaPolitica,
            "SELECT id, version, texto, vigente_desde, publicada_por, creada_en
             FROM politicas_publicidad WHERE vigente_desde <= $1
             ORDER BY version DESC LIMIT 1",
            ahora,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Into::into))
    }

    async fn autorizar_publicidad(
        &mut self,
        persona: PersonaId,
        politica: PoliticaPublicidadId,
        medio: MedioAutorizacion,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let n = sqlx::query!(
            "INSERT INTO autorizaciones_tratamiento
                 (id, tenant_id, persona_id, politica_publicidad_id, finalidad, medio, otorgada_en)
             VALUES ($1, $2, $3, $4, 'publicidad', $5, $6)
             ON CONFLICT (tenant_id, persona_id, finalidad) WHERE revocada_en IS NULL
             DO NOTHING",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            persona.as_uuid(),
            politica.as_uuid(),
            medio.as_str(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(n == 1)
    }

    async fn insertar_reporte_publicidad(&mut self, r: &ReportePublicidad) -> AppResult<bool> {
        let n = sqlx::query!(
            "INSERT INTO reportes_publicidad (id, tenant_id, creativo_id, cuenta_id, motivo,
                                              creado_en)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (tenant_id, creativo_id, cuenta_id) DO NOTHING",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.creativo_id.as_uuid(),
            r.cuenta_id.as_uuid(),
            r.motivo,
            r.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|d| d.is_foreign_key_violation())
            {
                AppError::NoEncontrado("anuncio")
            } else {
                AppError::infra(e)
            }
        })?
        .rows_affected();
        Ok(n == 1)
    }

    async fn destinatarios_masivos(
        &mut self,
        campana: CampanaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
        limite: u32,
    ) -> AppResult<Vec<DestinatarioMasivo>> {
        // `destinatarios_comunicado` ya excluye a los menores y exige la autorización vigente.
        let desde = ahora - Duration::days(FRECUENCIA_MASIVA_DIAS);
        let filas = sqlx::query!(
            r#"SELECT d.persona_id AS "persona_id!", d.telefono, d.correo
               FROM destinatarios_comunicado('{"tipo":"todos"}'::jsonb, 'publicidad', $1) d
               WHERE NOT EXISTS (
                   SELECT 1 FROM envios_publicidad e
                   WHERE e.persona_id = d.persona_id AND e.canal = $2
                     AND (e.campana_id = $3 OR e.enviado_en > $4))
                 AND CASE $2 WHEN 'whatsapp' THEN d.telefono IS NOT NULL
                             ELSE d.correo IS NOT NULL END
               ORDER BY d.persona_id LIMIT $5"#,
            ahora,
            canal.as_str(),
            campana.as_uuid(),
            desde,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(filas
            .into_iter()
            .map(|f| DestinatarioMasivo {
                persona_id: PersonaId::from_uuid(f.persona_id),
                telefono: f.telefono,
                correo: f.correo,
            })
            .collect())
    }

    async fn registrar_envio_publicidad(
        &mut self,
        campana: CampanaId,
        persona: PersonaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO envios_publicidad (tenant_id, campana_id, persona_id, canal, enviado_en)
             VALUES ($1, $2, $3, $4, $5)",
            self.ctx.tenant_id.as_uuid(),
            campana.as_uuid(),
            persona.as_uuid(),
            canal.as_str(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn encolar_publicidad(
        &mut self,
        evento: &EventoSaliente,
        campana: CampanaId,
    ) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto,
                                 campana_id)
             VALUES ($1, $2, $3, $4, $5, $6, 'publicidad', $7)",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
            campana.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn pantalla(&mut self) -> AppResult<Option<PantallaPublicidad>> {
        Ok(sqlx::query!(
            "SELECT token_hash, creado_por, creado_en FROM pantallas_publicidad
             WHERE tenant_id = $1",
            self.ctx.tenant_id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| PantallaPublicidad {
            token_hash: f.token_hash,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        }))
    }

    async fn guardar_pantalla(&mut self, p: &PantallaPublicidad) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO pantallas_publicidad (tenant_id, token_hash, creado_por, creado_en)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id) DO UPDATE
                 SET token_hash = EXCLUDED.token_hash, creado_por = EXCLUDED.creado_por,
                     creado_en = EXCLUDED.creado_en",
            self.ctx.tenant_id.as_uuid(),
            p.token_hash,
            p.creado_por.map(CuentaId::as_uuid),
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

/// Copropiedades con el módulo `publicidad` (worker de envíos masivos).
#[derive(Debug, Clone)]
pub struct DirectorioPublicidadPg {
    pool: PgPool,
}

impl DirectorioPublicidadPg {
    /// Directorio sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioPublicidad for DirectorioPublicidadPg {
    async fn copropiedades_con_publicidad(&self, ahora: DateTime<Utc>) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "t!" FROM copropiedades_con_publicidad($1)"#,
            ahora
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }
}
