//! Directorio de comercios y servicios (plan 08, fase 1; ADR 0013).
//!
//! - [`RepositorioDirectorioPlataforma`] sobre la transacción de plataforma: catálogo,
//!   anunciantes, fichas y la cola de moderación (`resenas_reportadas`).
//! - [`RepositorioDirectorio`] sobre la transacción del tenant: las fichas que alcanzan a la
//!   copropiedad salen de `fichas_directorio_copropiedad` (SECURITY DEFINER); las reseñas
//!   viven con RLS por tenant.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioDirectorio, RepositorioDirectorioPlataforma,
        directorio::{
            Anunciante, Categoria, Contacto, Ficha, FichaDirectorio, FiltroAnunciantes,
            FiltroDirectorio, OrganizacionAnunciante, ReporteResena, Resena, ResenaReportada,
            ResumenCalificacion,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, Calificacion, Cobertura, CodigoCategoria, CodigoDane, Coordenadas, CuentaId,
    DireccionCorreo, EstadoResena, EstadoVerificacion, OrganizacionId, ResenaId, Telefono,
    TenantId, TipoCobertura, Ubicacion,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPg, TransaccionPlataformaPg, plataforma::conflicto_si_unico};

fn codigos(v: &[String]) -> AppResult<Vec<CodigoCategoria>> {
    v.iter()
        .map(|c| CodigoCategoria::new(c).map_err(AppError::from))
        .collect()
}

fn textos(v: &[CodigoCategoria]) -> Vec<String> {
    v.iter().map(|c| c.as_str().to_owned()).collect()
}

fn ubicacion(
    latitud: Option<Decimal>,
    longitud: Option<Decimal>,
    municipio: Option<&str>,
) -> AppResult<Ubicacion> {
    let coordenadas = match (latitud, longitud) {
        (Some(la), Some(lo)) => Some(Coordenadas::new(la, lo)?),
        _ => None,
    };
    Ok(Ubicacion {
        coordenadas,
        municipio: municipio.map(CodigoDane::new).transpose()?,
    })
}

fn contacto(
    telefono: Option<&str>,
    whatsapp: Option<&str>,
    correo: Option<&str>,
    sitio_web: Option<String>,
) -> AppResult<Contacto> {
    Ok(Contacto {
        telefono: telefono.map(Telefono::new).transpose()?,
        whatsapp: whatsapp.map(Telefono::new).transpose()?,
        correo: correo.map(DireccionCorreo::new).transpose()?,
        sitio_web,
    })
}

fn contador(v: Option<i64>) -> u32 {
    u32::try_from(v.unwrap_or(0)).unwrap_or(u32::MAX)
}

fn calificacion(v: i16) -> AppResult<Calificacion> {
    Ok(Calificacion::new(u8::try_from(v).unwrap_or(0))?)
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

struct FilaCategoria {
    codigo: String,
    padre: Option<String>,
    nombre: String,
    sinonimos: Vec<String>,
    restringida: bool,
    activa: bool,
}

impl TryFrom<FilaCategoria> for Categoria {
    type Error = AppError;

    fn try_from(f: FilaCategoria) -> AppResult<Self> {
        Ok(Self {
            codigo: CodigoCategoria::new(&f.codigo)?,
            padre: f.padre.as_deref().map(CodigoCategoria::new).transpose()?,
            nombre: f.nombre,
            sinonimos: f.sinonimos,
            restringida: f.restringida,
            activa: f.activa,
        })
    }
}

fn categorias(filas: Vec<FilaCategoria>) -> AppResult<Vec<Categoria>> {
    filas.into_iter().map(Categoria::try_from).collect()
}

struct FilaAnunciante {
    id: Uuid,
    organizacion_id: Uuid,
    nombre_comercial: String,
    direccion: Option<String>,
    municipio_dane: Option<String>,
    latitud: Option<Decimal>,
    longitud: Option<Decimal>,
    estado_verificacion: String,
    motivo_estado: Option<String>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

impl TryFrom<FilaAnunciante> for Anunciante {
    type Error = AppError;

    fn try_from(f: FilaAnunciante) -> AppResult<Self> {
        Ok(Self {
            id: AnuncianteId::from_uuid(f.id),
            organizacion_id: OrganizacionId::from_uuid(f.organizacion_id),
            nombre_comercial: f.nombre_comercial,
            direccion: f.direccion,
            ubicacion: ubicacion(f.latitud, f.longitud, f.municipio_dane.as_deref())?,
            estado: EstadoVerificacion::try_from(f.estado_verificacion.as_str())?,
            motivo_estado: f.motivo_estado,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

impl TransaccionPlataformaPg {
    async fn leer_anunciante(
        &mut self,
        id: AnuncianteId,
        bloquear: bool,
    ) -> AppResult<Option<Anunciante>> {
        // Dos consultas en lugar de SQL dinámico: `FOR UPDATE` no se puede enlazar.
        let fila = if bloquear {
            sqlx::query_as!(
                FilaAnunciante,
                "SELECT id, organizacion_id, nombre_comercial, direccion, municipio_dane, latitud,
                        longitud, estado_verificacion, motivo_estado, creado_por, creado_en,
                        actualizado_en
                 FROM anunciantes WHERE id = $1 FOR UPDATE",
                id.as_uuid(),
            )
            .fetch_optional(&mut *self.tx)
            .await
        } else {
            sqlx::query_as!(
                FilaAnunciante,
                "SELECT id, organizacion_id, nombre_comercial, direccion, municipio_dane, latitud,
                        longitud, estado_verificacion, motivo_estado, creado_por, creado_en,
                        actualizado_en
                 FROM anunciantes WHERE id = $1",
                id.as_uuid(),
            )
            .fetch_optional(&mut *self.tx)
            .await
        };
        fila.map_err(AppError::infra)?
            .map(Anunciante::try_from)
            .transpose()
    }
}

impl RepositorioDirectorioPlataforma for TransaccionPlataformaPg {
    async fn categorias(&mut self) -> AppResult<Vec<Categoria>> {
        categorias(
            sqlx::query_as!(
                FilaCategoria,
                "SELECT codigo, padre, nombre, sinonimos, restringida, activa
                 FROM categorias_publicidad
                 ORDER BY coalesce(padre, codigo), padre NULLS FIRST, normalizar(nombre)",
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?,
        )
    }

    async fn categoria(&mut self, codigo: &CodigoCategoria) -> AppResult<Option<Categoria>> {
        sqlx::query_as!(
            FilaCategoria,
            "SELECT codigo, padre, nombre, sinonimos, restringida, activa
             FROM categorias_publicidad WHERE codigo = $1",
            codigo.as_str(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Categoria::try_from)
        .transpose()
    }

    async fn guardar_categoria(&mut self, c: &Categoria) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO categorias_publicidad (codigo, padre, nombre, sinonimos, restringida, activa)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (codigo) DO UPDATE
                 SET padre = EXCLUDED.padre, nombre = EXCLUDED.nombre,
                     sinonimos = EXCLUDED.sinonimos, restringida = EXCLUDED.restringida,
                     activa = EXCLUDED.activa",
            c.codigo.as_str(),
            c.padre.as_ref().map(CodigoCategoria::as_str),
            c.nombre,
            &c.sinonimos,
            c.restringida,
            c.activa,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn tiene_subcategorias(&mut self, codigo: &CodigoCategoria) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM categorias_publicidad WHERE padre = $1) AS "existe!""#,
            codigo.as_str(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn categorias_por_codigo(
        &mut self,
        codigos: &[CodigoCategoria],
    ) -> AppResult<Vec<Categoria>> {
        categorias(
            sqlx::query_as!(
                FilaCategoria,
                "SELECT codigo, padre, nombre, sinonimos, restringida, activa
                 FROM categorias_publicidad WHERE codigo = ANY($1)",
                &textos(codigos),
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?,
        )
    }

    async fn organizacion_para_anunciante(
        &mut self,
        organizacion: &OrganizacionAnunciante,
        creada_por: Option<CuentaId>,
    ) -> AppResult<Option<OrganizacionId>> {
        let id = match organizacion {
            OrganizacionAnunciante::Existente(id) => {
                sqlx::query_scalar!("SELECT id FROM organizaciones WHERE id = $1", id.as_uuid(),)
                    .fetch_optional(&mut *self.tx)
                    .await
                    .map_err(AppError::infra)?
            }
            OrganizacionAnunciante::Nueva(n) => {
                let tipos: Vec<String> = n.tipos.iter().map(|t| t.as_str().to_owned()).collect();
                let nit = n.nit.to_string();
                sqlx::query!(
                    "INSERT INTO organizaciones (id, nit, razon_social, naturaleza, tipos,
                                                 correo_contacto, telefono_contacto,
                                                 creada_por_cuenta)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                     ON CONFLICT (nit) DO NOTHING",
                    OrganizacionId::nuevo().as_uuid(),
                    nit,
                    n.razon_social.trim(),
                    n.naturaleza.as_str(),
                    &tipos,
                    n.correo_contacto.as_deref(),
                    n.telefono_contacto.as_deref(),
                    creada_por.map(CuentaId::as_uuid),
                )
                .execute(&mut *self.tx)
                .await
                .map_err(AppError::infra)?;
                sqlx::query_scalar!("SELECT id FROM organizaciones WHERE nit = $1", nit)
                    .fetch_optional(&mut *self.tx)
                    .await
                    .map_err(AppError::infra)?
            }
        };
        let Some(id) = id else {
            return Ok(None);
        };
        sqlx::query!(
            "UPDATE organizaciones
             SET tipos = array_append(tipos, 'anunciante'), actualizada_en = now()
             WHERE id = $1 AND NOT ('anunciante' = ANY (tipos))",
            id,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(Some(OrganizacionId::from_uuid(id)))
    }

    async fn insertar_anunciante(&mut self, a: &Anunciante) -> AppResult<()> {
        let c = a.ubicacion.coordenadas;
        sqlx::query!(
            "INSERT INTO anunciantes (id, organizacion_id, nombre_comercial, direccion,
                                      municipio_dane, latitud, longitud, estado_verificacion,
                                      motivo_estado, creado_por, creado_en, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            a.id.as_uuid(),
            a.organizacion_id.as_uuid(),
            a.nombre_comercial,
            a.direccion,
            a.ubicacion.municipio.as_ref().map(CodigoDane::as_str),
            c.map(Coordenadas::latitud),
            c.map(Coordenadas::longitud),
            a.estado.as_str(),
            a.motivo_estado,
            a.creado_por.map(CuentaId::as_uuid),
            a.creado_en,
            a.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "la organización ya tiene un anunciante"))?;
        Ok(())
    }

    async fn anunciante(&mut self, id: AnuncianteId) -> AppResult<Option<Anunciante>> {
        self.leer_anunciante(id, false).await
    }

    async fn anunciante_para_actualizar(
        &mut self,
        id: AnuncianteId,
    ) -> AppResult<Option<Anunciante>> {
        self.leer_anunciante(id, true).await
    }

    async fn anunciantes(
        &mut self,
        f: &FiltroAnunciantes,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Anunciante>> {
        let filas = sqlx::query_as!(
            FilaAnunciante,
            "SELECT id, organizacion_id, nombre_comercial, direccion, municipio_dane, latitud,
                    longitud, estado_verificacion, motivo_estado, creado_por, creado_en,
                    actualizado_en
             FROM anunciantes
             WHERE ($1::text IS NULL OR estado_verificacion = $1)
               AND ($2::text IS NULL OR normalizar(nombre_comercial) LIKE '%' || normalizar($2) || '%')
               AND ($3::uuid IS NULL OR id > $3)
             ORDER BY id LIMIT $4",
            f.estado.map(EstadoVerificacion::as_str),
            f.texto.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Anunciante::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |a| a.id.as_uuid()))
    }

    async fn actualizar_anunciante(&mut self, a: &Anunciante) -> AppResult<()> {
        let c = a.ubicacion.coordenadas;
        sqlx::query!(
            "UPDATE anunciantes
             SET nombre_comercial = $2, direccion = $3, municipio_dane = $4, latitud = $5,
                 longitud = $6, estado_verificacion = $7, motivo_estado = $8, actualizado_en = $9
             WHERE id = $1",
            a.id.as_uuid(),
            a.nombre_comercial,
            a.direccion,
            a.ubicacion.municipio.as_ref().map(CodigoDane::as_str),
            c.map(Coordenadas::latitud),
            c.map(Coordenadas::longitud),
            a.estado.as_str(),
            a.motivo_estado,
            a.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn ficha(&mut self, id: AnuncianteId) -> AppResult<Option<Ficha>> {
        let Some(f) = sqlx::query!(
            r#"SELECT f.descripcion, f.cobertura, f.radio_km, f.telefono, f.whatsapp, f.correo,
                      f.sitio_web, f.horario, f.destacada_hasta, f.publicada, f.actualizada_en,
                      coalesce((SELECT array_agg(fc.categoria ORDER BY fc.orden)
                                FROM ficha_categorias fc WHERE fc.anunciante_id = f.anunciante_id),
                               '{}') AS "categorias!"
               FROM fichas_directorio f WHERE f.anunciante_id = $1"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(Ficha {
            anunciante_id: id,
            descripcion: f.descripcion,
            categorias: codigos(&f.categorias)?,
            cobertura: Cobertura::new(
                TipoCobertura::try_from(f.cobertura.as_str())?,
                f.radio_km.and_then(|r| u16::try_from(r).ok()),
            )?,
            contacto: contacto(
                f.telefono.as_deref(),
                f.whatsapp.as_deref(),
                f.correo.as_deref(),
                f.sitio_web,
            )?,
            horario: f.horario,
            destacada_hasta: f.destacada_hasta,
            publicada: f.publicada,
            actualizada_en: f.actualizada_en,
        }))
    }

    async fn guardar_ficha(&mut self, f: &Ficha) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO fichas_directorio (anunciante_id, descripcion, cobertura, radio_km,
                                            telefono, whatsapp, correo, sitio_web, horario,
                                            destacada_hasta, publicada, actualizada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (anunciante_id) DO UPDATE
                 SET descripcion = EXCLUDED.descripcion, cobertura = EXCLUDED.cobertura,
                     radio_km = EXCLUDED.radio_km, telefono = EXCLUDED.telefono,
                     whatsapp = EXCLUDED.whatsapp, correo = EXCLUDED.correo,
                     sitio_web = EXCLUDED.sitio_web, horario = EXCLUDED.horario,
                     destacada_hasta = EXCLUDED.destacada_hasta, publicada = EXCLUDED.publicada,
                     actualizada_en = EXCLUDED.actualizada_en",
            f.anunciante_id.as_uuid(),
            f.descripcion,
            f.cobertura.tipo().as_str(),
            f.cobertura
                .radio_km()
                .map(|r| i16::try_from(r).unwrap_or(i16::MAX)),
            f.contacto.telefono.as_ref().map(Telefono::as_str),
            f.contacto.whatsapp.as_ref().map(Telefono::as_str),
            f.contacto.correo.as_ref().map(DireccionCorreo::as_str),
            f.contacto.sitio_web,
            f.horario,
            f.destacada_hasta,
            f.publicada,
            f.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "DELETE FROM ficha_categorias WHERE anunciante_id = $1",
            f.anunciante_id.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO ficha_categorias (anunciante_id, categoria, orden)
             SELECT $1, c, (o - 1)::smallint FROM unnest($2::text[]) WITH ORDINALITY AS t (c, o)",
            f.anunciante_id.as_uuid(),
            &textos(&f.categorias),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn resenas_reportadas(
        &mut self,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<ResenaReportada>> {
        let filas = sqlx::query!(
            r#"SELECT resena_id AS "resena_id!", tenant_id AS "tenant_id!",
                      copropiedad AS "copropiedad!", anunciante_id AS "anunciante_id!",
                      nombre_comercial AS "nombre_comercial!", calificacion AS "calificacion!",
                      texto, reportes, motivos AS "motivos!", creada_en AS "creada_en!"
               FROM resenas_reportadas($1, $2::int)"#,
            p.despues(),
            i32::try_from(p.filas_a_consultar()).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(|f| {
                Ok(ResenaReportada {
                    resena_id: ResenaId::from_uuid(f.resena_id),
                    tenant_id: TenantId::from_uuid(f.tenant_id),
                    copropiedad: f.copropiedad,
                    anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
                    nombre_comercial: f.nombre_comercial,
                    calificacion: calificacion(f.calificacion)?,
                    texto: f.texto,
                    reportes: contador(f.reportes),
                    motivos: f.motivos,
                    creada_en: f.creada_en,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |r| r.resena_id.as_uuid()))
    }
}

// ---------------------------------------------------------------------------------------------
// Copropiedad
// ---------------------------------------------------------------------------------------------

struct FilaResena {
    id: Uuid,
    anunciante_id: Uuid,
    cuenta_id: Uuid,
    calificacion: i16,
    texto: Option<String>,
    estado: String,
    creada_en: DateTime<Utc>,
    actualizada_en: DateTime<Utc>,
}

impl TryFrom<FilaResena> for Resena {
    type Error = AppError;

    fn try_from(f: FilaResena) -> AppResult<Self> {
        Ok(Self {
            id: ResenaId::from_uuid(f.id),
            anunciante_id: AnuncianteId::from_uuid(f.anunciante_id),
            cuenta_id: CuentaId::from_uuid(f.cuenta_id),
            calificacion: calificacion(f.calificacion)?,
            texto: f.texto,
            estado: EstadoResena::try_from(f.estado.as_str())?,
            creada_en: f.creada_en,
            actualizada_en: f.actualizada_en,
        })
    }
}

impl RepositorioDirectorio for TransaccionPg {
    async fn fichas_directorio(
        &mut self,
        f: &FiltroDirectorio,
        ahora: DateTime<Utc>,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<FichaDirectorio>> {
        let filas = sqlx::query!(
            r#"SELECT anunciante_id AS "anunciante_id!", nombre_comercial AS "nombre_comercial!",
                      descripcion, categorias, telefono, whatsapp, correo, sitio_web, horario,
                      direccion, municipio_dane, destacada AS "destacada!", distancia_km,
                      calificacion_promedio, resenas
               FROM fichas_directorio_copropiedad($1, $2, $3, $4, $5, $6, $7::int)"#,
            f.anunciante.map(AnuncianteId::as_uuid),
            f.categoria.as_ref().map(CodigoCategoria::as_str),
            f.texto.as_deref(),
            f.solo_destacadas,
            ahora,
            p.despues(),
            i32::try_from(p.filas_a_consultar()).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(|r| {
                Ok(FichaDirectorio {
                    anunciante_id: AnuncianteId::from_uuid(r.anunciante_id),
                    nombre_comercial: r.nombre_comercial,
                    descripcion: r.descripcion,
                    categorias: codigos(&r.categorias.unwrap_or_default())?,
                    contacto: contacto(
                        r.telefono.as_deref(),
                        r.whatsapp.as_deref(),
                        r.correo.as_deref(),
                        r.sitio_web,
                    )?,
                    horario: r.horario,
                    direccion: r.direccion,
                    municipio: r
                        .municipio_dane
                        .as_deref()
                        .map(CodigoDane::new)
                        .transpose()?,
                    destacada: r.destacada,
                    distancia_km: r.distancia_km,
                    calificacion: ResumenCalificacion {
                        promedio: r.calificacion_promedio,
                        total: contador(r.resenas),
                    },
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |x| x.anunciante_id.as_uuid()))
    }

    async fn categorias_directorio(&mut self) -> AppResult<Vec<Categoria>> {
        categorias(
            sqlx::query_as!(
                FilaCategoria,
                "SELECT codigo, padre, nombre, sinonimos, restringida, activa
                 FROM categorias_publicidad WHERE activa AND NOT restringida
                 ORDER BY coalesce(padre, codigo), padre NULLS FIRST, normalizar(nombre)",
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?,
        )
    }

    async fn es_residente_vigente(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM asignaciones_rol a JOIN roles r ON r.id = a.rol_id
                   WHERE a.cuenta_id = $1 AND r.es_plantilla
                     AND r.codigo IN ('propietario', 'residente')
                     AND a.origen IN ('propiedad', 'residencia')
                     AND a.vigente_desde <= $2
                     AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)) AS "vigente!""#,
            cuenta.as_uuid(),
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn resena_de_cuenta(
        &mut self,
        anunciante: AnuncianteId,
        cuenta: CuentaId,
    ) -> AppResult<Option<Resena>> {
        sqlx::query_as!(
            FilaResena,
            "SELECT id, anunciante_id, cuenta_id, calificacion, texto, estado, creada_en,
                    actualizada_en
             FROM resenas WHERE anunciante_id = $1 AND cuenta_id = $2 FOR UPDATE",
            anunciante.as_uuid(),
            cuenta.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Resena::try_from)
        .transpose()
    }

    async fn resena(&mut self, id: ResenaId) -> AppResult<Option<Resena>> {
        sqlx::query_as!(
            FilaResena,
            "SELECT id, anunciante_id, cuenta_id, calificacion, texto, estado, creada_en,
                    actualizada_en
             FROM resenas WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Resena::try_from)
        .transpose()
    }

    async fn guardar_resena(&mut self, r: &Resena) -> AppResult<()> {
        // Dos reseñas simultáneas de la misma cuenta: la segunda actualiza la primera.
        sqlx::query!(
            "INSERT INTO resenas (id, tenant_id, anunciante_id, cuenta_id, calificacion, texto,
                                  estado, creada_en, actualizada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id, anunciante_id, cuenta_id) DO UPDATE
                 SET calificacion = EXCLUDED.calificacion, texto = EXCLUDED.texto,
                     actualizada_en = EXCLUDED.actualizada_en
                 WHERE resenas.estado = 'publicada'",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.anunciante_id.as_uuid(),
            r.cuenta_id.as_uuid(),
            i16::from(r.calificacion.valor()),
            r.texto,
            r.estado.as_str(),
            r.creada_en,
            r.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn resenas_publicadas(
        &mut self,
        anunciante: AnuncianteId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Resena>> {
        let filas = sqlx::query_as!(
            FilaResena,
            "SELECT id, anunciante_id, cuenta_id, calificacion, texto, estado, creada_en,
                    actualizada_en
             FROM resenas
             WHERE anunciante_id = $1 AND estado = 'publicada' AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
            anunciante.as_uuid(),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Resena::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |r| r.id.as_uuid()))
    }

    async fn insertar_reporte(&mut self, r: &ReporteResena) -> AppResult<bool> {
        let insertadas = sqlx::query!(
            "INSERT INTO reportes_resena (tenant_id, resena_id, cuenta_id, motivo, creado_en)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id, resena_id, cuenta_id) DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            r.resena_id.as_uuid(),
            r.cuenta_id.as_uuid(),
            r.motivo,
            r.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(insertadas == 1)
    }

    async fn moderar_resena(
        &mut self,
        id: ResenaId,
        ocultar_con_motivo: Option<&str>,
        moderador: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(motivo) = ocultar_con_motivo {
            sqlx::query!(
                "UPDATE resenas
                 SET estado = 'oculta', motivo_moderacion = $2, moderada_por = $3, moderada_en = $4
                 WHERE id = $1",
                id.as_uuid(),
                motivo,
                moderador.map(CuentaId::as_uuid),
                ahora,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        sqlx::query!(
            "UPDATE reportes_resena SET atendido_en = $2 WHERE resena_id = $1 AND atendido_en IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
