//! Censo sobre la transacción del tenant: residencias, vehículos, mascotas, propuestas de
//! cambio y los libros de propietarios y residentes.

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioCenso,
        censo::{
            CambioCenso, FilaLibroPropietarios, FilaLibroResidentes, Mascota, Residencia, Vehiculo,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CalidadResidencia, CambioCensoId, CodigoBienPrivado, CuentaId,
    Documento, EspecieMascota, EstadoCambioCenso, EstadoMascota, MascotaId, PersonaId, Placa,
    Porcentaje, ResidenciaId, TipoCambioCenso, TipoDocumento, TipoTitularidad, TipoVehiculo,
    VehiculoId,
};
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{error_escritura, opcional},
};

struct FilaResidencia {
    id: Uuid,
    bien_privado_id: Uuid,
    persona_id: Uuid,
    calidad: String,
    es_responsable: bool,
    autorizado_por: Option<Uuid>,
    contrato_archivo_id: Option<Uuid>,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
}

impl TryFrom<FilaResidencia> for Residencia {
    type Error = AppError;

    fn try_from(f: FilaResidencia) -> AppResult<Self> {
        Ok(Self {
            id: ResidenciaId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            calidad: CalidadResidencia::try_from(f.calidad.as_str())?,
            es_responsable: f.es_responsable,
            autorizado_por: f.autorizado_por.map(PersonaId::from_uuid),
            contrato_archivo_id: f.contrato_archivo_id.map(ArchivoId::from_uuid),
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
        })
    }
}

struct FilaVehiculo {
    id: Uuid,
    bien_privado_id: Uuid,
    persona_id: Option<Uuid>,
    tipo: String,
    placa: Option<String>,
    marca: Option<String>,
    color: Option<String>,
    tarjeta_archivo_id: Option<Uuid>,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
}

impl TryFrom<FilaVehiculo> for Vehiculo {
    type Error = AppError;

    fn try_from(f: FilaVehiculo) -> AppResult<Self> {
        Ok(Self {
            id: VehiculoId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            persona_id: f.persona_id.map(PersonaId::from_uuid),
            tipo: TipoVehiculo::try_from(f.tipo.as_str())?,
            placa: opcional(f.placa, Placa::new)?,
            marca: f.marca,
            color: f.color,
            tarjeta_archivo_id: f.tarjeta_archivo_id.map(ArchivoId::from_uuid),
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
        })
    }
}

struct FilaMascota {
    id: Uuid,
    bien_privado_id: Uuid,
    responsable_id: Option<Uuid>,
    especie: String,
    nombre: String,
    raza: Option<String>,
    potencialmente_peligrosa: bool,
    poliza_numero: Option<String>,
    poliza_vence: Option<NaiveDate>,
    poliza_archivo_id: Option<Uuid>,
    vacunas_vencen: Option<NaiveDate>,
    carne_archivo_id: Option<Uuid>,
    esterilizada: bool,
    foto_archivo_id: Option<Uuid>,
    estado: String,
}

impl TryFrom<FilaMascota> for Mascota {
    type Error = AppError;

    fn try_from(f: FilaMascota) -> AppResult<Self> {
        Ok(Self {
            id: MascotaId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            responsable_id: f.responsable_id.map(PersonaId::from_uuid),
            especie: EspecieMascota::try_from(f.especie.as_str())?,
            nombre: f.nombre,
            raza: f.raza,
            potencialmente_peligrosa: f.potencialmente_peligrosa,
            poliza_numero: f.poliza_numero,
            poliza_vence: f.poliza_vence,
            poliza_archivo_id: f.poliza_archivo_id.map(ArchivoId::from_uuid),
            vacunas_vencen: f.vacunas_vencen,
            carne_archivo_id: f.carne_archivo_id.map(ArchivoId::from_uuid),
            esterilizada: f.esterilizada,
            foto_archivo_id: f.foto_archivo_id.map(ArchivoId::from_uuid),
            estado: EstadoMascota::try_from(f.estado.as_str())?,
        })
    }
}

struct FilaCambio {
    id: Uuid,
    bien_privado_id: Uuid,
    tipo: String,
    datos: serde_json::Value,
    estado: String,
    propuesto_por: Uuid,
    propuesto_en: DateTime<Utc>,
    revisado_por: Option<Uuid>,
    revisado_en: Option<DateTime<Utc>>,
    motivo: Option<String>,
    resultado_id: Option<Uuid>,
}

impl TryFrom<FilaCambio> for CambioCenso {
    type Error = AppError;

    fn try_from(f: FilaCambio) -> AppResult<Self> {
        Ok(Self {
            id: CambioCensoId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            tipo: TipoCambioCenso::try_from(f.tipo.as_str())?,
            datos: f.datos,
            estado: EstadoCambioCenso::try_from(f.estado.as_str())?,
            propuesto_por: CuentaId::from_uuid(f.propuesto_por),
            propuesto_en: f.propuesto_en,
            revisado_por: f.revisado_por.map(CuentaId::from_uuid),
            revisado_en: f.revisado_en,
            motivo: f.motivo,
            resultado_id: f.resultado_id,
        })
    }
}

impl RepositorioCenso for TransaccionPg {
    async fn residencias(
        &mut self,
        bien: Option<BienPrivadoId>,
        incluir_terminadas: bool,
    ) -> AppResult<Vec<Residencia>> {
        sqlx::query_as!(
            FilaResidencia,
            "SELECT id, bien_privado_id, persona_id, calidad, es_responsable, autorizado_por,
                    contrato_archivo_id, vigente_desde, vigente_hasta
             FROM residencias
             WHERE ($1::uuid IS NULL OR bien_privado_id = $1) AND ($2 OR vigente_hasta IS NULL)
             ORDER BY vigente_desde DESC, id",
            bien.map(BienPrivadoId::as_uuid),
            incluir_terminadas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Residencia::try_from)
        .collect()
    }

    async fn residencia(&mut self, id: ResidenciaId) -> AppResult<Option<Residencia>> {
        sqlx::query_as!(
            FilaResidencia,
            "SELECT id, bien_privado_id, persona_id, calidad, es_responsable, autorizado_por,
                    contrato_archivo_id, vigente_desde, vigente_hasta
             FROM residencias WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Residencia::try_from)
        .transpose()
    }

    async fn insertar_residencia(&mut self, r: &Residencia) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO residencias (id, tenant_id, bien_privado_id, persona_id, calidad,
                                      es_responsable, autorizado_por, contrato_archivo_id,
                                      vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.bien_privado_id.as_uuid(),
            r.persona_id.as_uuid(),
            r.calidad.as_str(),
            r.es_responsable,
            r.autorizado_por.map(PersonaId::as_uuid),
            r.contrato_archivo_id.map(ArchivoId::as_uuid),
            r.vigente_desde,
            r.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "la persona ya reside allí o el bien ya tiene responsable",
                "bien privado, persona o archivo",
            )
        })?;
        Ok(())
    }

    async fn terminar_residencia(
        &mut self,
        id: ResidenciaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE residencias SET vigente_hasta = greatest($2, vigente_desde)
             WHERE id = $1 AND vigente_hasta IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn vehiculos(
        &mut self,
        bien: Option<BienPrivadoId>,
        placa: Option<&Placa>,
        incluir_retirados: bool,
    ) -> AppResult<Vec<Vehiculo>> {
        sqlx::query_as!(
            FilaVehiculo,
            "SELECT id, bien_privado_id, persona_id, tipo, placa, marca, color, tarjeta_archivo_id,
                    vigente_desde, vigente_hasta
             FROM vehiculos
             WHERE ($1::uuid IS NULL OR bien_privado_id = $1)
               AND ($2::text IS NULL OR placa = $2)
               AND ($3 OR vigente_hasta IS NULL)
             ORDER BY vigente_desde DESC, id",
            bien.map(BienPrivadoId::as_uuid),
            placa.map(Placa::as_str),
            incluir_retirados,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Vehiculo::try_from)
        .collect()
    }

    async fn vehiculo(&mut self, id: VehiculoId) -> AppResult<Option<Vehiculo>> {
        sqlx::query_as!(
            FilaVehiculo,
            "SELECT id, bien_privado_id, persona_id, tipo, placa, marca, color, tarjeta_archivo_id,
                    vigente_desde, vigente_hasta
             FROM vehiculos WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Vehiculo::try_from)
        .transpose()
    }

    async fn insertar_vehiculo(&mut self, v: &Vehiculo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO vehiculos (id, tenant_id, bien_privado_id, persona_id, tipo, placa, marca,
                                    color, tarjeta_archivo_id, vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            v.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            v.bien_privado_id.as_uuid(),
            v.persona_id.map(PersonaId::as_uuid),
            v.tipo.as_str(),
            v.placa.as_ref().map(Placa::as_str),
            v.marca,
            v.color,
            v.tarjeta_archivo_id.map(ArchivoId::as_uuid),
            v.vigente_desde,
            v.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "esa placa ya está registrada en la copropiedad",
                "bien privado, persona o archivo",
            )
        })?;
        Ok(())
    }

    async fn retirar_vehiculo(&mut self, id: VehiculoId, ahora: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "UPDATE vehiculos SET vigente_hasta = greatest($2, vigente_desde)
             WHERE id = $1 AND vigente_hasta IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn mascotas(
        &mut self,
        bien: Option<BienPrivadoId>,
        solo_activas: bool,
    ) -> AppResult<Vec<Mascota>> {
        sqlx::query_as!(
            FilaMascota,
            "SELECT id, bien_privado_id, responsable_id, especie, nombre, raza,
                    potencialmente_peligrosa, poliza_numero, poliza_vence, poliza_archivo_id,
                    vacunas_vencen, carne_archivo_id, esterilizada, foto_archivo_id, estado
             FROM mascotas
             WHERE ($1::uuid IS NULL OR bien_privado_id = $1) AND (NOT $2 OR estado = 'activa')
             ORDER BY nombre, id",
            bien.map(BienPrivadoId::as_uuid),
            solo_activas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Mascota::try_from)
        .collect()
    }

    async fn mascota(&mut self, id: MascotaId) -> AppResult<Option<Mascota>> {
        sqlx::query_as!(
            FilaMascota,
            "SELECT id, bien_privado_id, responsable_id, especie, nombre, raza,
                    potencialmente_peligrosa, poliza_numero, poliza_vence, poliza_archivo_id,
                    vacunas_vencen, carne_archivo_id, esterilizada, foto_archivo_id, estado
             FROM mascotas WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Mascota::try_from)
        .transpose()
    }

    async fn insertar_mascota(&mut self, m: &Mascota) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO mascotas (id, tenant_id, bien_privado_id, responsable_id, especie, nombre,
                                   raza, potencialmente_peligrosa, poliza_numero, poliza_vence,
                                   poliza_archivo_id, vacunas_vencen, carne_archivo_id,
                                   esterilizada, foto_archivo_id, estado)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            m.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            m.bien_privado_id.as_uuid(),
            m.responsable_id.map(PersonaId::as_uuid),
            m.especie.as_str(),
            m.nombre,
            m.raza,
            m.potencialmente_peligrosa,
            m.poliza_numero,
            m.poliza_vence,
            m.poliza_archivo_id.map(ArchivoId::as_uuid),
            m.vacunas_vencen,
            m.carne_archivo_id.map(ArchivoId::as_uuid),
            m.esterilizada,
            m.foto_archivo_id.map(ArchivoId::as_uuid),
            m.estado.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "bien privado, persona o archivo"))?;
        Ok(())
    }

    async fn actualizar_mascota(&mut self, m: &Mascota) -> AppResult<()> {
        sqlx::query!(
            "UPDATE mascotas SET
                 responsable_id = $2, raza = $3, potencialmente_peligrosa = $4,
                 poliza_numero = $5, poliza_vence = $6, poliza_archivo_id = $7,
                 vacunas_vencen = $8, carne_archivo_id = $9, esterilizada = $10,
                 foto_archivo_id = $11, estado = $12, actualizado_en = now()
             WHERE id = $1",
            m.id.as_uuid(),
            m.responsable_id.map(PersonaId::as_uuid),
            m.raza,
            m.potencialmente_peligrosa,
            m.poliza_numero,
            m.poliza_vence,
            m.poliza_archivo_id.map(ArchivoId::as_uuid),
            m.vacunas_vencen,
            m.carne_archivo_id.map(ArchivoId::as_uuid),
            m.esterilizada,
            m.foto_archivo_id.map(ArchivoId::as_uuid),
            m.estado.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "persona o archivo"))?;
        Ok(())
    }

    async fn cambios(
        &mut self,
        estado: Option<EstadoCambioCenso>,
        propuesto_por: Option<CuentaId>,
    ) -> AppResult<Vec<CambioCenso>> {
        sqlx::query_as!(
            FilaCambio,
            "SELECT id, bien_privado_id, tipo, datos, estado, propuesto_por, propuesto_en,
                    revisado_por, revisado_en, motivo, resultado_id
             FROM cambios_censo
             WHERE ($1::text IS NULL OR estado = $1) AND ($2::uuid IS NULL OR propuesto_por = $2)
             ORDER BY propuesto_en DESC, id",
            estado.map(EstadoCambioCenso::as_str),
            propuesto_por.map(CuentaId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(CambioCenso::try_from)
        .collect()
    }

    async fn cambio(&mut self, id: CambioCensoId) -> AppResult<Option<CambioCenso>> {
        sqlx::query_as!(
            FilaCambio,
            "SELECT id, bien_privado_id, tipo, datos, estado, propuesto_por, propuesto_en,
                    revisado_por, revisado_en, motivo, resultado_id
             FROM cambios_censo WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(CambioCenso::try_from)
        .transpose()
    }

    async fn insertar_cambio(&mut self, c: &CambioCenso) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cambios_censo (id, tenant_id, bien_privado_id, tipo, datos, estado,
                                        propuesto_por, propuesto_en)
             VALUES ($1, $2, $3, $4, $5, 'pendiente', $6, $7)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.bien_privado_id.as_uuid(),
            c.tipo.as_str(),
            c.datos,
            c.propuesto_por.as_uuid(),
            c.propuesto_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "bien privado"))?;
        Ok(())
    }

    async fn resolver_cambio(&mut self, c: &CambioCenso) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cambios_censo
             SET estado = $2, revisado_por = $3, revisado_en = $4, motivo = $5, resultado_id = $6
             WHERE id = $1",
            c.id.as_uuid(),
            c.estado.as_str(),
            c.revisado_por.map(CuentaId::as_uuid),
            c.revisado_en,
            c.motivo,
            c.resultado_id,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "cambio del censo"))?;
        Ok(())
    }

    async fn cuenta_vinculada_a_bien(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM personas p
                   WHERE p.cuenta_id = $1 AND (
                       EXISTS (SELECT 1 FROM titularidades t
                               WHERE t.persona_id = p.id AND t.bien_privado_id = $2
                                 AND t.vigente_hasta IS NULL)
                       OR EXISTS (SELECT 1 FROM residencias r
                                  WHERE r.persona_id = p.id AND r.bien_privado_id = $2
                                    AND r.vigente_hasta IS NULL))
               ) AS "vinculada!""#,
            cuenta.as_uuid(),
            bien.as_uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn libro_propietarios(
        &mut self,
        corte: DateTime<Utc>,
    ) -> AppResult<Vec<FilaLibroPropietarios>> {
        sqlx::query!(
            "WITH cuadro AS (
                 SELECT id FROM cuadros_coeficientes
                 WHERE tipo = 'copropiedad' AND estado <> 'borrador' AND vigente_desde <= $1
                 ORDER BY vigente_desde DESC LIMIT 1)
             SELECT b.codigo, cb.coeficiente AS \"coeficiente?\", p.nombre, p.tipo_documento,
                    p.documento, t.tipo, t.porcentaje, t.es_representante, t.vigente_desde
             FROM titularidades t
             JOIN bienes_privados b ON b.id = t.bien_privado_id
             JOIN personas p        ON p.id = t.persona_id
             LEFT JOIN coeficientes_bien_privado cb
                    ON cb.bien_privado_id = b.id AND cb.cuadro_id = (SELECT id FROM cuadro)
             WHERE t.vigente_desde <= $1 AND (t.vigente_hasta IS NULL OR t.vigente_hasta > $1)
             ORDER BY normalizar(b.codigo), t.es_representante DESC, normalizar(p.nombre)",
            corte,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(FilaLibroPropietarios {
                codigo: CodigoBienPrivado::new(&f.codigo)?,
                coeficiente: f.coeficiente,
                nombre: f.nombre,
                tipo_documento: opcional(f.tipo_documento, |t| TipoDocumento::try_from(t))?,
                documento: opcional(f.documento, Documento::new)?,
                tipo: TipoTitularidad::try_from(f.tipo.as_str())?,
                porcentaje: Porcentaje::new(f.porcentaje)?,
                es_representante: f.es_representante,
                vigente_desde: f.vigente_desde,
            })
        })
        .collect()
    }

    async fn libro_residentes(
        &mut self,
        corte: DateTime<Utc>,
    ) -> AppResult<Vec<FilaLibroResidentes>> {
        sqlx::query!(
            "SELECT b.codigo, p.nombre, p.tipo_documento, p.documento, r.calidad,
                    r.es_responsable, r.vigente_desde
             FROM residencias r
             JOIN bienes_privados b ON b.id = r.bien_privado_id
             JOIN personas p        ON p.id = r.persona_id
             WHERE r.vigente_desde <= $1 AND (r.vigente_hasta IS NULL OR r.vigente_hasta > $1)
             ORDER BY normalizar(b.codigo), r.es_responsable DESC, normalizar(p.nombre)",
            corte,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(FilaLibroResidentes {
                codigo: CodigoBienPrivado::new(&f.codigo)?,
                nombre: f.nombre,
                tipo_documento: opcional(f.tipo_documento, |t| TipoDocumento::try_from(t))?,
                documento: opcional(f.documento, Documento::new)?,
                calidad: CalidadResidencia::try_from(f.calidad.as_str())?,
                es_responsable: f.es_responsable,
                vigente_desde: f.vigente_desde,
            })
        })
        .collect()
    }

    async fn residentes_sin_autorizacion(&mut self) -> AppResult<Vec<PersonaId>> {
        Ok(sqlx::query_scalar!(
            "SELECT DISTINCT r.persona_id FROM residencias r
             WHERE r.vigente_hasta IS NULL
               AND NOT EXISTS (SELECT 1 FROM autorizaciones_tratamiento a
                               WHERE a.persona_id = r.persona_id AND a.finalidad = 'administracion'
                                 AND a.revocada_en IS NULL)
             ORDER BY 1"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(PersonaId::from_uuid)
        .collect())
    }
}
