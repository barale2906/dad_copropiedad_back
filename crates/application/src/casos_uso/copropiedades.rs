//! Ficha de la copropiedad: datos legales y de contacto, cargos vigentes, advertencias legales
//! (arts. 53 y 56) y puntos de atención con su horario.
//!
//! El alta y el ciclo de vida (activar, suspender, retirar) los hace la plataforma
//! ([`crate::casos_uso::plataforma`]).

use chrono::NaiveDate;
use domain::{
    ArchivoId, CodigoDane, DireccionCorreo, EstadoLegal, FormaCopropiedad, Horario, Nit, Permiso,
    PlantillaNomenclatura, PuntoAtencionId, Telefono, TipoCopropiedad, TipoPuntoAtencion,
    copropiedad::{AdvertenciaFicha, texto_obligatorio, texto_opcional},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, RepositorioCoeficientes, RepositorioCopropiedad, Transaccion,
        UnidadDeTrabajo,
        copropiedades::{CargoVigente, FichaCopropiedad, PuntoAtencion},
    },
};

pub(crate) const ACCION_ACTUALIZAR: AccionAuditoria =
    AccionAuditoria::new("copropiedad.datos.actualizar");
pub(crate) const ACCION_PUNTO_ATENCION: AccionAuditoria =
    AccionAuditoria::new("copropiedad.punto_atencion.crear");
pub(crate) const ACCION_HORARIO: AccionAuditoria =
    AccionAuditoria::new("copropiedad.punto_atencion.definir_horario");

/// Ficha completa: datos, cargos vigentes y advertencias legales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaCompleta {
    pub datos: FichaCopropiedad,
    pub cargos: Vec<CargoVigente>,
    pub advertencias: Vec<AdvertenciaFicha>,
}

/// Ficha de la copropiedad del contexto.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.ver`; [`AppError::NoEncontrado`].
pub async fn ver_ficha<U>(unidad: &U, ctx: &Contexto) -> AppResult<FichaCompleta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad + RepositorioCoeficientes,
{
    ctx.exigir(Permiso::CopropiedadDatosVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let datos = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    let cargos = tx.cargos_vigentes().await?;
    let legal = EstadoLegal {
        tipo: datos.tipo,
        tiene_revisor_fiscal: cargos.iter().any(|c| c.rol == "revisor_fiscal"),
        tiene_consejo: cargos
            .iter()
            .any(|c| c.rol == "consejero" || c.rol == "presidente_consejo"),
        bienes_para_consejo: tx.bienes_para_consejo().await?,
        tiene_cuadro_vigente: tx.cuadro_vigente().await?.is_some(),
    };
    Ok(FichaCompleta {
        advertencias: legal.advertencias(),
        datos,
        cargos,
    })
}

/// Cargos vigentes (administrador primero).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.ver`.
pub async fn cargos_vigentes<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<CargoVigente>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad,
{
    ctx.exigir(Permiso::CopropiedadDatosVer)?;
    unidad.iniciar(ctx).await?.cargos_vigentes().await
}

/// Cambios a la ficha. `None` = sin cambio. En los textos, `Some("")` borra el valor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosCopropiedad {
    pub nombre: Option<String>,
    pub razon_social: Option<String>,
    pub nit: Option<Nit>,
    pub tipo: Option<TipoCopropiedad>,
    pub forma: Option<FormaCopropiedad>,
    pub direccion: Option<String>,
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    pub latitud: Option<Decimal>,
    pub longitud: Option<Decimal>,
    pub estrato: Option<u8>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    pub escritura_numero: Option<String>,
    pub notaria: Option<String>,
    pub escritura_fecha: Option<NaiveDate>,
    pub matricula_matriz: Option<String>,
    pub reglamento_archivo_id: Option<ArchivoId>,
    pub certificado_archivo_id: Option<ArchivoId>,
    pub zona_horaria: Option<String>,
    pub plantilla_codigo: Option<PlantillaNomenclatura>,
}

fn coordenada(campo: &'static str, v: Decimal, limite: Decimal) -> AppResult<Decimal> {
    if v.abs() > limite || v.normalize().scale() > 6 {
        return Err(AppError::Validacion(format!(
            "`{campo}` debe estar entre -{limite} y {limite} con máximo 6 decimales"
        )));
    }
    Ok(v)
}

fn texto(
    actual: &mut Option<String>,
    campo: &'static str,
    nuevo: Option<String>,
    maximo: usize,
) -> AppResult<()> {
    if let Some(v) = nuevo {
        *actual = texto_opcional(campo, Some(&v), maximo)?;
    }
    Ok(())
}

impl CambiosCopropiedad {
    /// Aplica los cambios sobre la ficha validando cada campo.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] o [`AppError::Dominio`] con el primer campo inválido.
    pub fn aplicar(self, f: &mut FichaCopropiedad) -> AppResult<()> {
        if let Some(n) = self.nombre {
            f.nombre = texto_obligatorio("nombre", &n, 200)?;
        }
        texto(&mut f.razon_social, "razon_social", self.razon_social, 200)?;
        texto(&mut f.direccion, "direccion", self.direccion, 200)?;
        texto(&mut f.departamento, "departamento", self.departamento, 80)?;
        texto(&mut f.barrio, "barrio", self.barrio, 120)?;
        texto(
            &mut f.escritura_numero,
            "escritura_numero",
            self.escritura_numero,
            40,
        )?;
        texto(&mut f.notaria, "notaria", self.notaria, 120)?;
        texto(
            &mut f.matricula_matriz,
            "matricula_matriz",
            self.matricula_matriz,
            40,
        )?;
        texto(&mut f.sitio_web, "sitio_web", self.sitio_web, 200)?;
        if f.sitio_web
            .as_deref()
            .is_some_and(|s| !(s.starts_with("https://") || s.starts_with("http://")))
        {
            return Err(AppError::Validacion(
                "`sitio_web` debe empezar por https://".into(),
            ));
        }
        if let Some(z) = self.zona_horaria {
            let z = z.trim();
            let valida = z.split('/').count() >= 2
                && z.split('/').all(|p| {
                    !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
                });
            if !valida {
                return Err(AppError::Validacion(
                    "`zona_horaria` debe ser una zona IANA, p. ej. America/Bogota".into(),
                ));
            }
            z.clone_into(&mut f.zona_horaria);
        }
        if let Some(e) = self.estrato {
            if !(1..=6).contains(&e) {
                return Err(AppError::Validacion("`estrato` va de 1 a 6".into()));
            }
            f.estrato = Some(e);
        }
        if let Some(v) = self.latitud {
            f.latitud = Some(coordenada("latitud", v, Decimal::from(90))?);
        }
        if let Some(v) = self.longitud {
            f.longitud = Some(coordenada("longitud", v, Decimal::from(180))?);
        }
        if let Some(v) = self.nit {
            f.nit = Some(v);
        }
        if let Some(v) = self.tipo {
            f.tipo = v;
        }
        if let Some(v) = self.forma {
            f.forma = v;
        }
        if let Some(v) = self.municipio_dane {
            f.municipio_dane = Some(v);
        }
        if let Some(v) = self.telefono {
            f.telefono = Some(v);
        }
        if let Some(v) = self.correo {
            f.correo = Some(v);
        }
        if let Some(v) = self.escritura_fecha {
            f.escritura_fecha = Some(v);
        }
        if let Some(v) = self.reglamento_archivo_id {
            f.reglamento_archivo_id = Some(v);
        }
        if let Some(v) = self.certificado_archivo_id {
            f.certificado_archivo_id = Some(v);
        }
        if let Some(v) = self.plantilla_codigo {
            f.plantilla_codigo = v;
        }
        Ok(())
    }
}

/// Actualiza los datos de la copropiedad. Cambiar la plantilla de nomenclatura no recalcula
/// los códigos ya asignados (son la identificación que conoce la gente).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.editar`; errores de validación;
/// [`AppError::Conflicto`] si el NIT ya está registrado en otra copropiedad.
pub async fn actualizar_datos<U>(
    unidad: &U,
    ctx: &Contexto,
    cambios: CambiosCopropiedad,
) -> AppResult<FichaCopropiedad>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::CopropiedadDatosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    cambios.aplicar(&mut ficha)?;
    tx.guardar_ficha(&ficha).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ACTUALIZAR,
        "copropiedad",
        ctx.tenant_id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(ficha)
}

/// Puntos de atención con su horario.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.ver`.
pub async fn listar_puntos_atencion<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<PuntoAtencion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad,
{
    ctx.exigir(Permiso::CopropiedadDatosVer)?;
    unidad.iniciar(ctx).await?.puntos_atencion().await
}

/// Datos de un punto de atención nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoPuntoAtencion {
    pub tipo: TipoPuntoAtencion,
    pub nombre: String,
    pub ubicacion: Option<String>,
    pub telefono: Option<Telefono>,
    pub horario: Horario,
}

/// Registra un punto de atención (oficina de administración, portería…).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.editar`; errores de validación.
pub async fn registrar_punto_atencion<U>(
    unidad: &U,
    ctx: &Contexto,
    nuevo: NuevoPuntoAtencion,
) -> AppResult<PuntoAtencion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::CopropiedadDatosEditar)?;
    let punto = PuntoAtencion {
        id: PuntoAtencionId::nuevo(),
        tipo: nuevo.tipo,
        nombre: texto_obligatorio("nombre", &nuevo.nombre, 80)?,
        ubicacion: texto_opcional("ubicacion", nuevo.ubicacion.as_deref(), 200)?,
        telefono: nuevo.telefono,
        horario: nuevo.horario,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar_punto_atencion(&punto).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PUNTO_ATENCION,
        "punto_atencion",
        punto.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(punto)
}

/// Reemplaza el horario de un punto de atención.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.editar`; [`AppError::NoEncontrado`].
pub async fn definir_horario<U>(
    unidad: &U,
    ctx: &Contexto,
    punto: PuntoAtencionId,
    horario: Horario,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::CopropiedadDatosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx.reemplazar_horario(punto, &horario).await? {
        return Err(AppError::NoEncontrado("punto de atención"));
    }
    let detalle = DetalleAuditoria::vacio().con("franjas", horario.franjas().len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_HORARIO, "punto_atencion", punto.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

#[cfg(test)]
mod tests {
    use domain::EstadoCopropiedad;
    use rust_decimal::dec;

    use super::*;

    fn ficha() -> FichaCopropiedad {
        FichaCopropiedad {
            nombre: "Conjunto".into(),
            razon_social: None,
            nit: None,
            tipo: TipoCopropiedad::Residencial,
            forma: FormaCopropiedad::Conjunto,
            direccion: Some("Calle 1".into()),
            municipio_dane: None,
            departamento: None,
            barrio: None,
            latitud: None,
            longitud: None,
            estrato: None,
            telefono: None,
            correo: None,
            sitio_web: None,
            escritura_numero: None,
            notaria: None,
            escritura_fecha: None,
            matricula_matriz: None,
            reglamento_archivo_id: None,
            certificado_archivo_id: None,
            zona_horaria: "America/Bogota".into(),
            estado: EstadoCopropiedad::EnConfiguracion,
            plantilla_codigo: PlantillaNomenclatura::default(),
        }
    }

    #[test]
    fn aplica_cambios_y_texto_vacio_borra() {
        let mut f = ficha();
        CambiosCopropiedad {
            nombre: Some(" Conjunto Alameda ".into()),
            direccion: Some(String::new()),
            latitud: Some(dec!(4.711)),
            estrato: Some(4),
            ..Default::default()
        }
        .aplicar(&mut f)
        .expect("válidos");
        assert_eq!(f.nombre, "Conjunto Alameda");
        assert_eq!(f.direccion, None);
        assert_eq!(f.latitud, Some(dec!(4.711)));
    }

    #[test]
    fn rechaza_datos_invalidos() {
        let casos = [
            CambiosCopropiedad {
                nombre: Some(" ".into()),
                ..Default::default()
            },
            CambiosCopropiedad {
                estrato: Some(7),
                ..Default::default()
            },
            CambiosCopropiedad {
                latitud: Some(dec!(91)),
                ..Default::default()
            },
            CambiosCopropiedad {
                sitio_web: Some("ftp://x".into()),
                ..Default::default()
            },
            CambiosCopropiedad {
                zona_horaria: Some("Bogota".into()),
                ..Default::default()
            },
        ];
        for c in casos {
            assert!(c.clone().aplicar(&mut ficha()).is_err(), "{c:?}");
        }
    }
}
