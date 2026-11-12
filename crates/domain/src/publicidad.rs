//! Publicidad local de la plataforma (plan 08, fases 3 a 6; ADR 0015).
//!
//! **La publicidad es un negocio de la plataforma.** Las copropiedades no venden, no cobran ni
//! configuran nada: solo son el canal para llegar a los residentes que **autorizaron** recibir
//! publicidad (finalidad `publicidad`, frente a la plataforma). Reglas de este módulo:
//!
//! - Ciclos de vida: [`EstadoCampana::transicionar`], [`EstadoCreativo::transicionar`],
//!   [`EstadoLiquidacion::transicionar`].
//! - [`Segmentacion::alcanza`]: ¿la campaña llega a una copropiedad? Solo con el perfil de la
//!   PH (ubicación, tipo, tamaño, estrato), **nunca** con atributos de personas.
//! - [`costo`] y [`ajustar_al_presupuesto`]: modelos de cobro y topes de presupuesto (total y
//!   diario), que detienen la entrega.
//! - [`seleccionar_piezas`]: qué piezas se muestran (anunciante verificado, creativo aprobado,
//!   campaña activa y vigente, con presupuesto), rotando las menos vistas del día.
//! - [`puede_recibir_masivo`]: tope de frecuencia de los mensajes masivos.
//! - [`ContenidoCreativo::validado`]: las promociones declaran sus condiciones (Ley 1480 de 2011).

use chrono::{DateTime, Duration, Utc};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};

use crate::{
    CampanaId, CodigoDane, CreativoId, DominioError, RolOrganizacion, TenantId, TipoCopropiedad,
    directorio::{Cobertura, RADIO_MAXIMO_KM, Ubicacion, cubre},
};

/// Días mínimos entre dos mensajes masivos de publicidad a la misma persona por canal.
pub const FRECUENCIA_MASIVA_DIAS: i64 = 7;
/// Máximo de copropiedades elegidas una por una en una segmentación.
pub const MAXIMO_COPROPIEDADES: usize = 500;

fn transicion(entidad: &'static str, desde: &'static str, hacia: &'static str) -> DominioError {
    DominioError::TransicionInvalida {
        entidad,
        desde,
        hacia,
    }
}

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

// ---------------------------------------------------------------------------------------------
// Ciclos de vida
// ---------------------------------------------------------------------------------------------

/// Estado de una campaña.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCampana {
    Borrador,
    EnRevision,
    Aprobada,
    Rechazada,
    Activa,
    Pausada,
    Finalizada,
}

enum_texto!(EstadoCampana {
    Borrador => "borrador",
    EnRevision => "en_revision",
    Aprobada => "aprobada",
    Rechazada => "rechazada",
    Activa => "activa",
    Pausada => "pausada",
    Finalizada => "finalizada",
});

impl EstadoCampana {
    /// `borrador → en_revision → aprobada | rechazada`, `rechazada → borrador`,
    /// `aprobada → activa | borrador`, `activa ⇄ pausada`, `pausada → borrador` y
    /// `aprobada | activa | pausada → finalizada`. Volver a `borrador` (para editar) obliga a
    /// pasar otra vez por la moderación.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoCampana::*;
        match (self, hacia) {
            (Borrador, EnRevision)
            | (EnRevision, Aprobada | Rechazada)
            | (Rechazada | Aprobada | Pausada, Borrador)
            | (Aprobada | Pausada, Activa)
            | (Activa, Pausada)
            | (Aprobada | Activa | Pausada, Finalizada) => Ok(hacia),
            _ => Err(transicion("campaña", self.as_str(), hacia.as_str())),
        }
    }

    /// ¿Se pueden cambiar sus datos, su segmentación y sus creativos?
    pub const fn editable(self) -> bool {
        matches!(self, Self::Borrador | Self::Rechazada)
    }
}

/// Estado de moderación de un creativo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCreativo {
    Pendiente,
    Aprobado,
    Rechazado,
}

enum_texto!(EstadoCreativo {
    Pendiente => "pendiente",
    Aprobado => "aprobado",
    Rechazado => "rechazado",
});

impl EstadoCreativo {
    /// `pendiente → aprobado | rechazado`; `aprobado → pendiente` (cambió el contenido) o
    /// `rechazado` (un reporte lo retira); `rechazado → pendiente` (corregido).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoCreativo::*;
        match (self, hacia) {
            (Pendiente, Aprobado | Rechazado)
            | (Aprobado, Pendiente | Rechazado)
            | (Rechazado, Pendiente) => Ok(hacia),
            _ => Err(transicion("creativo", self.as_str(), hacia.as_str())),
        }
    }
}

/// Estado de una liquidación de pauta (sin reparto a las copropiedades).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoLiquidacion {
    Calculada,
    Facturada,
    Pagada,
}

enum_texto!(EstadoLiquidacion {
    Calculada => "calculada",
    Facturada => "facturada",
    Pagada => "pagada",
});

impl EstadoLiquidacion {
    /// `calculada → facturada → pagada`. Solo una `calculada` se puede recalcular.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoLiquidacion::*;
        match (self, hacia) {
            (Calculada, Facturada) | (Facturada, Pagada) => Ok(hacia),
            _ => Err(transicion("liquidación", self.as_str(), hacia.as_str())),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Catálogos
// ---------------------------------------------------------------------------------------------

/// Objetivo de una campaña.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjetivoCampana {
    Visibilidad,
    Trafico,
    Cupones,
    Leads,
}

enum_texto!(ObjetivoCampana {
    Visibilidad => "visibilidad",
    Trafico => "trafico",
    Cupones => "cupones",
    Leads => "leads",
});

/// Modelo de cobro de una campaña.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeloCobro {
    /// Valor fijo por periodo (se cobra en la liquidación del mes en que estuvo activa).
    TarifaFija,
    /// Por mil impresiones (los mensajes masivos entregados cuentan como impresiones).
    Cpm,
    /// Por clic.
    Cpc,
    /// Por cupón redimido.
    PorRedencion,
    /// Por contacto autorizado desde la búsqueda de proveedores.
    PorLead,
    /// Suscripción mensual de la ficha destacada.
    DirectorioDestacado,
}

enum_texto!(ModeloCobro {
    TarifaFija => "tarifa_fija",
    Cpm => "cpm",
    Cpc => "cpc",
    PorRedencion => "por_redencion",
    PorLead => "por_lead",
    DirectorioDestacado => "directorio_destacado",
});

impl ModeloCobro {
    /// ¿Se cobra por periodo (no por evento)?
    pub const fn es_fijo(self) -> bool {
        matches!(self, Self::TarifaFija | Self::DirectorioDestacado)
    }
}

/// Canal por el que se entrega una pieza.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalPublicidad {
    /// App (Flutter) y web de los residentes.
    App,
    /// Pantalla del lobby de la copropiedad.
    Pantalla,
    Whatsapp,
    Correo,
}

enum_texto!(CanalPublicidad {
    App => "app",
    Pantalla => "pantalla",
    Whatsapp => "whatsapp",
    Correo => "correo",
});

/// Formato de un creativo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatoCreativo {
    Banner,
    TarjetaDestacada,
    /// Oferta con cupón (código que el comercio redime).
    OfertaCupon,
    /// Plantilla de marketing aprobada en Meta, solo a quien autorizó.
    MensajeWhatsapp,
    /// Boletín por correo, solo a quien autorizó.
    BoletinCorreo,
    PantallaLobby,
}

enum_texto!(FormatoCreativo {
    Banner => "banner",
    TarjetaDestacada => "tarjeta_destacada",
    OfertaCupon => "oferta_cupon",
    MensajeWhatsapp => "mensaje_whatsapp",
    BoletinCorreo => "boletin_correo",
    PantallaLobby => "pantalla_lobby",
});

impl FormatoCreativo {
    /// Canal por el que sale el formato.
    pub const fn canal(self) -> CanalPublicidad {
        match self {
            Self::Banner | Self::TarjetaDestacada | Self::OfertaCupon => CanalPublicidad::App,
            Self::MensajeWhatsapp => CanalPublicidad::Whatsapp,
            Self::BoletinCorreo => CanalPublicidad::Correo,
            Self::PantallaLobby => CanalPublicidad::Pantalla,
        }
    }

    /// ¿Se envía a personas (masivo) en vez de mostrarse cuando alguien abre la app?
    pub const fn es_masivo(self) -> bool {
        matches!(self, Self::MensajeWhatsapp | Self::BoletinCorreo)
    }
}

/// Qué puede hacer un miembro de la organización anunciante en su portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccionAnunciante {
    /// Ficha, campañas y creativos.
    Gestionar,
    /// Métricas agregadas de sus campañas.
    VerMetricas,
    /// Contactos autorizados por los residentes.
    VerLeads,
    /// Redimir cupones en el mostrador.
    RedimirCupones,
}

/// Permisos del portal según el papel en la organización: dueño y gestor hacen todo; el
/// operario solo redime cupones.
pub const fn permite(rol: RolOrganizacion, accion: AccionAnunciante) -> bool {
    match rol {
        RolOrganizacion::Dueno | RolOrganizacion::Gestor => true,
        RolOrganizacion::Operario => matches!(accion, AccionAnunciante::RedimirCupones),
    }
}

// ---------------------------------------------------------------------------------------------
// Segmentación (solo atributos de la copropiedad)
// ---------------------------------------------------------------------------------------------

/// Perfil de una copropiedad para segmentar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfilCopropiedad {
    pub id: TenantId,
    pub ubicacion: Ubicacion,
    pub tipo: TipoCopropiedad,
    /// Bienes privados activos.
    pub bienes: u32,
    pub estrato: Option<u8>,
}

/// A qué copropiedades llega una campaña. Cada criterio vacío no restringe; todos se cumplen a
/// la vez. **Nunca** atributos de personas.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Segmentacion {
    /// Copropiedades elegidas una por una (vacío = todas las que cumplan lo demás).
    pub copropiedades: Vec<TenantId>,
    /// Radio en km desde el anunciante (sin coordenadas, el mismo municipio).
    pub radio_km: Option<u16>,
    pub municipios: Vec<CodigoDane>,
    pub tipos: Vec<TipoCopropiedad>,
    pub bienes_min: Option<u32>,
    pub bienes_max: Option<u32>,
    pub estratos: Vec<u8>,
}

impl Segmentacion {
    /// Radio de 1 a [`RADIO_MAXIMO_KM`], estratos de 1 a 6, rango de bienes coherente y hasta
    /// [`MAXIMO_COPROPIEDADES`] copropiedades. Quita repetidos.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si algún criterio no es válido.
    pub fn validada(mut self) -> Result<Self, DominioError> {
        if self
            .radio_km
            .is_some_and(|r| !(1..=RADIO_MAXIMO_KM).contains(&r))
        {
            return Err(invalido("radio_km", "de 1 a 100 km"));
        }
        if self.estratos.iter().any(|e| !(1..=6).contains(e)) {
            return Err(invalido("estratos", "de 1 a 6"));
        }
        if let (Some(a), Some(b)) = (self.bienes_min, self.bienes_max)
            && a > b
        {
            return Err(invalido("bienes", "el mínimo supera al máximo"));
        }
        self.copropiedades.sort();
        self.copropiedades.dedup();
        if self.copropiedades.len() > MAXIMO_COPROPIEDADES {
            return Err(invalido("copropiedades", "hasta 500 copropiedades"));
        }
        self.municipios.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        self.municipios.dedup();
        self.tipos.sort_by_key(|t| t.as_str());
        self.tipos.dedup();
        self.estratos.sort_unstable();
        self.estratos.dedup();
        Ok(self)
    }

    /// ¿La campaña de un anunciante ubicado en `anunciante` llega a la copropiedad? (La misma
    /// regla aplica la función SQL `campana_alcanza`.)
    pub fn alcanza(&self, anunciante: &Ubicacion, ph: &PerfilCopropiedad) -> bool {
        (self.copropiedades.is_empty() || self.copropiedades.contains(&ph.id))
            && self
                .radio_km
                .is_none_or(|r| cubre(Cobertura::Radio(r), anunciante, &ph.ubicacion))
            && (self.municipios.is_empty()
                || ph
                    .ubicacion
                    .municipio
                    .as_ref()
                    .is_some_and(|m| self.municipios.contains(m)))
            && (self.tipos.is_empty() || self.tipos.contains(&ph.tipo))
            && self.bienes_min.is_none_or(|m| ph.bienes >= m)
            && self.bienes_max.is_none_or(|m| ph.bienes <= m)
            && (self.estratos.is_empty() || ph.estrato.is_some_and(|e| self.estratos.contains(&e)))
    }
}

// ---------------------------------------------------------------------------------------------
// Cobro y presupuesto
// ---------------------------------------------------------------------------------------------

/// Eventos de una entrega (agregados: nunca identifican a la persona).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Conteo {
    pub impresiones: u32,
    pub clics: u32,
    pub redenciones: u32,
    pub leads: u32,
}

impl Conteo {
    /// Si no hubo impresiones, clics, redenciones ni leads.
    pub const fn vacio(&self) -> bool {
        self.impresiones == 0 && self.clics == 0 && self.redenciones == 0 && self.leads == 0
    }
}

/// Costo variable de unos eventos según el modelo y la tarifa (COP). Los modelos fijos se
/// cobran por periodo en la liquidación: aquí cuestan 0.
pub fn costo(modelo: ModeloCobro, tarifa: Decimal, c: Conteo) -> Decimal {
    match modelo {
        ModeloCobro::Cpm => tarifa * Decimal::from(c.impresiones) / Decimal::ONE_THOUSAND,
        ModeloCobro::Cpc => tarifa * Decimal::from(c.clics),
        ModeloCobro::PorRedencion => tarifa * Decimal::from(c.redenciones),
        ModeloCobro::PorLead => tarifa * Decimal::from(c.leads),
        ModeloCobro::TarifaFija | ModeloCobro::DirectorioDestacado => Decimal::ZERO,
    }
}

/// Presupuesto de una campaña (COP). El total es obligatorio; el diario, opcional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presupuesto {
    pub total: Decimal,
    pub diario: Option<Decimal>,
}

impl Presupuesto {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el total no es positivo o el diario no está entre 0 y
    /// el total.
    pub fn new(total: Decimal, diario: Option<Decimal>) -> Result<Self, DominioError> {
        if total <= Decimal::ZERO {
            return Err(invalido("presupuesto_total", "debe ser mayor que cero"));
        }
        if diario.is_some_and(|d| d <= Decimal::ZERO || d > total) {
            return Err(invalido(
                "presupuesto_diario",
                "debe ser mayor que cero y no superar el total",
            ));
        }
        Ok(Self { total, diario })
    }

    /// Lo que queda por gastar hoy (nunca negativo).
    pub fn disponible(&self, consumido_total: Decimal, consumido_hoy: Decimal) -> Decimal {
        let total = self.total - consumido_total;
        let hoy = self.diario.map_or(total, |d| d - consumido_hoy);
        total.min(hoy).max(Decimal::ZERO)
    }
}

/// Costo de un evento del contador que cobra el modelo (`None` si el modelo no cobra eventos).
fn costo_unitario(modelo: ModeloCobro, tarifa: Decimal) -> Option<Decimal> {
    match modelo {
        ModeloCobro::Cpm => Some(tarifa / Decimal::ONE_THOUSAND),
        ModeloCobro::Cpc | ModeloCobro::PorRedencion | ModeloCobro::PorLead => Some(tarifa),
        ModeloCobro::TarifaFija | ModeloCobro::DirectorioDestacado => None,
    }
    .filter(|u| *u > Decimal::ZERO)
}

/// Recorta el contador que cobra el modelo a lo que cabe en `disponible`; los demás eventos
/// pasan (no cuestan). Así una entrega concurrente nunca supera el tope.
pub fn ajustar_al_presupuesto(
    modelo: ModeloCobro,
    tarifa: Decimal,
    pedido: Conteo,
    disponible: Decimal,
) -> Conteo {
    let Some(unidad) = costo_unitario(modelo, tarifa) else {
        return pedido;
    };
    let caben = (disponible.max(Decimal::ZERO) / unidad)
        .floor()
        .to_u32()
        .unwrap_or(u32::MAX);
    let mut c = pedido;
    match modelo {
        ModeloCobro::Cpm => c.impresiones = c.impresiones.min(caben),
        ModeloCobro::Cpc => c.clics = c.clics.min(caben),
        ModeloCobro::PorRedencion => c.redenciones = c.redenciones.min(caben),
        ModeloCobro::PorLead => c.leads = c.leads.min(caben),
        ModeloCobro::TarifaFija | ModeloCobro::DirectorioDestacado => {}
    }
    c
}

/// ¿El presupuesto **total** ya no alcanza ni para un evento más? (Entonces la campaña se
/// finaliza; el tope diario solo la detiene hasta el día siguiente.)
pub fn presupuesto_agotado(
    modelo: ModeloCobro,
    tarifa: Decimal,
    presupuesto: &Presupuesto,
    consumido_total: Decimal,
) -> bool {
    costo_unitario(modelo, tarifa).is_some_and(|u| presupuesto.total - consumido_total < u)
}

// ---------------------------------------------------------------------------------------------
// Selección de piezas
// ---------------------------------------------------------------------------------------------

/// Pieza que podría mostrarse en una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiezaCandidata {
    pub creativo: CreativoId,
    pub campana: CampanaId,
    pub anunciante_verificado: bool,
    pub creativo_aprobado: bool,
    pub estado_campana: EstadoCampana,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    pub modelo: ModeloCobro,
    pub tarifa: Decimal,
    pub presupuesto: Presupuesto,
    pub consumido_total: Decimal,
    pub consumido_hoy: Decimal,
    /// Impresiones de hoy en esta copropiedad (para rotar).
    pub impresiones_hoy: u32,
}

impl PiezaCandidata {
    /// Invariante 4 del plan 08: anunciante verificado, creativo aprobado, campaña activa y
    /// vigente; y con presupuesto para al menos un evento (invariante 6).
    pub fn entregable(&self, ahora: DateTime<Utc>) -> bool {
        self.anunciante_verificado
            && self.creativo_aprobado
            && self.estado_campana == EstadoCampana::Activa
            && self.inicio <= ahora
            && ahora < self.fin
            && costo_unitario(self.modelo, self.tarifa).is_none_or(|u| {
                self.presupuesto
                    .disponible(self.consumido_total, self.consumido_hoy)
                    >= u
            })
    }
}

/// Las piezas que se muestran: solo las entregables, primero las menos vistas hoy en la
/// copropiedad (rotación), a lo sumo `maximo` y una por campaña.
pub fn seleccionar_piezas(
    mut candidatas: Vec<PiezaCandidata>,
    ahora: DateTime<Utc>,
    maximo: usize,
) -> Vec<PiezaCandidata> {
    candidatas.retain(|p| p.entregable(ahora));
    candidatas.sort_by(|a, b| {
        a.impresiones_hoy
            .cmp(&b.impresiones_hoy)
            .then(a.creativo.cmp(&b.creativo))
    });
    let mut vistas = Vec::new();
    candidatas.retain(|p| {
        let nueva = !vistas.contains(&p.campana);
        vistas.push(p.campana);
        nueva
    });
    candidatas.truncate(maximo);
    candidatas
}

/// Tope de frecuencia: un mensaje masivo de publicidad por persona y canal cada
/// [`FRECUENCIA_MASIVA_DIAS`] días.
pub fn puede_recibir_masivo(ultimo: Option<DateTime<Utc>>, ahora: DateTime<Utc>) -> bool {
    ultimo.is_none_or(|u| ahora - u >= Duration::days(FRECUENCIA_MASIVA_DIAS))
}

// ---------------------------------------------------------------------------------------------
// Creativos y cupones
// ---------------------------------------------------------------------------------------------

/// Código de un cupón: de 4 a 20 letras o dígitos, en mayúsculas.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoCupon(String);

impl CodigoCupon {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no tiene de 4 a 20 letras (A–Z) o dígitos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim().to_ascii_uppercase();
        if (4..=20).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric()) {
            Ok(Self(v))
        } else {
            Err(invalido("codigo", "de 4 a 20 letras o dígitos"))
        }
    }

    /// Código del cupón normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoCupon {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoCupon> for String {
    fn from(c: CodigoCupon) -> Self {
        c.0
    }
}

/// Cupón de una oferta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cupon {
    pub codigo: CodigoCupon,
    /// `None` = sin límite de usos.
    pub limite_usos: Option<u32>,
}

/// ¿Se puede redimir otra vez?
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si la campaña no está vigente o se agotaron los usos.
pub fn validar_redencion(cupon: &Cupon, usos: u32, vigente: bool) -> Result<(), DominioError> {
    if !vigente {
        return Err(invalido("codigo", "la oferta no está vigente"));
    }
    if cupon.limite_usos.is_some_and(|l| usos >= l) {
        return Err(invalido("codigo", "el cupón ya no tiene usos disponibles"));
    }
    Ok(())
}

/// Contenido de un creativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContenidoCreativo {
    pub formato: FormatoCreativo,
    pub titulo: String,
    pub texto: String,
    /// Texto del botón de acción.
    pub boton: Option<String>,
    /// Solo `https://`.
    pub enlace: Option<String>,
    /// Condiciones de la oferta (obligatorias en promociones y cupones, Ley 1480).
    pub condiciones: Option<String>,
    pub es_promocion: bool,
    /// Solo en [`FormatoCreativo::OfertaCupon`] (obligatorio allí).
    pub cupon: Option<Cupon>,
    /// Plantilla de marketing aprobada en Meta (obligatoria en
    /// [`FormatoCreativo::MensajeWhatsapp`]).
    pub plantilla_whatsapp: Option<String>,
}

fn texto(
    campo: &'static str,
    v: &str,
    max: usize,
    regla: &'static str,
) -> Result<String, DominioError> {
    let t = v.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(invalido(campo, regla));
    }
    Ok(t.to_owned())
}

fn opcional(
    campo: &'static str,
    v: Option<&str>,
    max: usize,
    regla: &'static str,
) -> Result<Option<String>, DominioError> {
    v.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| texto(campo, t, max, regla))
        .transpose()
}

impl ContenidoCreativo {
    /// Textos dentro de sus límites, enlace `https://`, condiciones en promociones y cupones
    /// (Ley 1480 de 2011), cupón solo en ofertas y plantilla en los mensajes de WhatsApp.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] con el campo que falla.
    pub fn validado(self) -> Result<Self, DominioError> {
        let titulo = texto("titulo", &self.titulo, 60, "de 1 a 60 caracteres")?;
        let cuerpo = texto("texto", &self.texto, 500, "de 1 a 500 caracteres")?;
        let boton = opcional("boton", self.boton.as_deref(), 20, "hasta 20 caracteres")?;
        let enlace = opcional(
            "enlace",
            self.enlace.as_deref(),
            300,
            "hasta 300 caracteres",
        )?;
        if enlace.as_ref().is_some_and(|e| !e.starts_with("https://")) {
            return Err(invalido("enlace", "debe empezar por https://"));
        }
        let condiciones = opcional(
            "condiciones",
            self.condiciones.as_deref(),
            1000,
            "hasta 1000 caracteres",
        )?;
        let cupon_obligatorio = self.formato == FormatoCreativo::OfertaCupon;
        if cupon_obligatorio != self.cupon.is_some() {
            return Err(invalido(
                "cupon",
                "las ofertas con cupón llevan código y los demás formatos no",
            ));
        }
        if (self.es_promocion || cupon_obligatorio) && condiciones.is_none() {
            return Err(invalido(
                "condiciones",
                "una promoción debe indicar sus condiciones y vigencia (Ley 1480)",
            ));
        }
        if self
            .cupon
            .as_ref()
            .is_some_and(|c| c.limite_usos == Some(0))
        {
            return Err(invalido("limite_usos", "debe ser mayor que cero"));
        }
        let plantilla = opcional(
            "plantilla_whatsapp",
            self.plantilla_whatsapp.as_deref(),
            512,
            "hasta 512 caracteres",
        )?;
        let es_whatsapp = self.formato == FormatoCreativo::MensajeWhatsapp;
        if es_whatsapp != plantilla.is_some()
            || plantilla.as_ref().is_some_and(|p| {
                !p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
        {
            return Err(invalido(
                "plantilla_whatsapp",
                "los mensajes de WhatsApp llevan el nombre de una plantilla aprobada (a-z, 0-9, _)",
            ));
        }
        Ok(Self {
            titulo,
            texto: cuerpo,
            boton,
            enlace,
            condiciones,
            plantilla_whatsapp: plantilla,
            es_promocion: self.es_promocion || cupon_obligatorio,
            ..self
        })
    }
}

/// Liquidación de un anunciante en un periodo: consumo variable (ya valorado al entregar),
/// cargos fijos de las campañas por periodo y contactos autorizados.
pub fn valor_liquidacion(
    variable: Decimal,
    fijos: Decimal,
    leads: u32,
    tarifa_lead: Decimal,
) -> Decimal {
    (variable + fijos + tarifa_lead * Decimal::from(leads)).round_dp(2)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal::dec;

    use super::*;
    use crate::{CodigoDane, Coordenadas};

    fn ahora() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, 12, 0, 0)
            .single()
            .expect("fecha")
    }

    #[test]
    fn campana_sigue_el_diagrama() {
        use EstadoCampana::*;
        assert_eq!(Borrador.transicionar(EnRevision), Ok(EnRevision));
        assert_eq!(EnRevision.transicionar(Aprobada), Ok(Aprobada));
        assert_eq!(Aprobada.transicionar(Activa), Ok(Activa));
        assert_eq!(Activa.transicionar(Pausada), Ok(Pausada));
        assert_eq!(Pausada.transicionar(Activa), Ok(Activa));
        assert_eq!(Pausada.transicionar(Borrador), Ok(Borrador));
        assert_eq!(Activa.transicionar(Finalizada), Ok(Finalizada));
        assert!(
            Borrador.transicionar(Activa).is_err(),
            "sin moderación no se activa"
        );
        assert!(EnRevision.transicionar(Activa).is_err());
        assert!(Rechazada.transicionar(Activa).is_err());
        assert!(Finalizada.transicionar(Activa).is_err());
        assert!(Activa.transicionar(Borrador).is_err(), "primero se pausa");
        assert!(Borrador.editable() && Rechazada.editable() && !Activa.editable());
    }

    #[test]
    fn creativo_y_liquidacion_siguen_el_diagrama() {
        use EstadoCreativo::*;
        assert_eq!(Pendiente.transicionar(Aprobado), Ok(Aprobado));
        assert_eq!(Aprobado.transicionar(Pendiente), Ok(Pendiente));
        assert_eq!(Aprobado.transicionar(Rechazado), Ok(Rechazado));
        assert!(Rechazado.transicionar(Aprobado).is_err());
        use EstadoLiquidacion as L;
        assert_eq!(L::Calculada.transicionar(L::Facturada), Ok(L::Facturada));
        assert!(L::Calculada.transicionar(L::Pagada).is_err());
        assert!(L::Pagada.transicionar(L::Calculada).is_err());
    }

    #[test]
    fn el_operario_solo_redime_cupones() {
        use AccionAnunciante::*;
        assert!(permite(RolOrganizacion::Gestor, VerLeads));
        assert!(permite(RolOrganizacion::Dueno, Gestionar));
        assert!(permite(RolOrganizacion::Operario, RedimirCupones));
        assert!(!permite(RolOrganizacion::Operario, Gestionar));
        assert!(!permite(RolOrganizacion::Operario, VerLeads));
    }

    fn ph(estrato: Option<u8>, bienes: u32) -> PerfilCopropiedad {
        PerfilCopropiedad {
            id: TenantId::nuevo(),
            ubicacion: Ubicacion {
                coordenadas: Some(Coordenadas::new(dec!(4.66), dec!(-74.06)).expect("válidas")),
                municipio: Some(CodigoDane::new("11001").expect("válido")),
            },
            tipo: TipoCopropiedad::Residencial,
            bienes,
            estrato,
        }
    }

    fn tienda() -> Ubicacion {
        Ubicacion {
            coordenadas: Some(Coordenadas::new(dec!(4.65), dec!(-74.06)).expect("válidas")),
            municipio: Some(CodigoDane::new("11001").expect("válido")),
        }
    }

    #[test]
    fn la_segmentacion_usa_solo_el_perfil_de_la_ph() {
        let todo = Segmentacion::default();
        let p = ph(Some(4), 120);
        assert!(todo.alcanza(&tienda(), &p));
        let cerca = Segmentacion {
            radio_km: Some(2),
            estratos: vec![3, 4],
            bienes_min: Some(100),
            ..Segmentacion::default()
        };
        assert!(cerca.alcanza(&tienda(), &p));
        assert!(!cerca.alcanza(&tienda(), &ph(Some(5), 120)), "otro estrato");
        assert!(!cerca.alcanza(&tienda(), &ph(None, 120)), "sin estrato");
        assert!(!cerca.alcanza(&tienda(), &ph(Some(4), 50)), "muy pequeña");
        let lejos = Segmentacion {
            radio_km: Some(1),
            ..Segmentacion::default()
        };
        assert!(!lejos.alcanza(&tienda(), &p), "≈1,1 km");
        let una = Segmentacion {
            copropiedades: vec![p.id],
            ..Segmentacion::default()
        };
        assert!(una.alcanza(&tienda(), &p));
        assert!(!una.alcanza(&tienda(), &ph(Some(4), 120)));
    }

    #[test]
    fn segmentacion_invalida_se_rechaza() {
        let s = |f: fn(&mut Segmentacion)| {
            let mut x = Segmentacion::default();
            f(&mut x);
            x.validada()
        };
        assert!(s(|x| x.radio_km = Some(0)).is_err());
        assert!(s(|x| x.radio_km = Some(101)).is_err());
        assert!(s(|x| x.estratos = vec![7]).is_err());
        assert!(
            s(|x| {
                x.bienes_min = Some(10);
                x.bienes_max = Some(5);
            })
            .is_err()
        );
        let ok = s(|x| x.estratos = vec![3, 3, 2]).expect("válida");
        assert_eq!(ok.estratos, [2, 3]);
    }

    #[test]
    fn costo_por_modelo() {
        let c = Conteo {
            impresiones: 2500,
            clics: 3,
            redenciones: 2,
            leads: 1,
        };
        assert_eq!(costo(ModeloCobro::Cpm, dec!(20000), c), dec!(50000));
        assert_eq!(costo(ModeloCobro::Cpc, dec!(800), c), dec!(2400));
        assert_eq!(costo(ModeloCobro::PorRedencion, dec!(1500), c), dec!(3000));
        assert_eq!(costo(ModeloCobro::PorLead, dec!(5000), c), dec!(5000));
        assert_eq!(
            costo(ModeloCobro::TarifaFija, dec!(90000), c),
            Decimal::ZERO
        );
    }

    #[test]
    fn el_presupuesto_recorta_lo_que_no_cabe() {
        let pedido = Conteo {
            impresiones: 10,
            clics: 5,
            ..Conteo::default()
        };
        let c = ajustar_al_presupuesto(ModeloCobro::Cpc, dec!(1000), pedido, dec!(2500));
        assert_eq!(
            (c.clics, c.impresiones),
            (2, 10),
            "las impresiones no cuestan en CPC"
        );
        let c = ajustar_al_presupuesto(ModeloCobro::Cpm, dec!(20000), pedido, dec!(100));
        assert_eq!(c.impresiones, 5);
        let c = ajustar_al_presupuesto(ModeloCobro::TarifaFija, dec!(1), pedido, Decimal::ZERO);
        assert_eq!(c, pedido);
        let p = Presupuesto::new(dec!(10000), Some(dec!(3000))).expect("válido");
        assert_eq!(p.disponible(dec!(2000), dec!(1000)), dec!(2000));
        assert_eq!(p.disponible(dec!(9000), dec!(0)), dec!(1000));
        assert_eq!(p.disponible(dec!(11000), dec!(0)), Decimal::ZERO);
        assert!(presupuesto_agotado(
            ModeloCobro::Cpc,
            dec!(1000),
            &p,
            dec!(9500)
        ));
        assert!(!presupuesto_agotado(
            ModeloCobro::Cpc,
            dec!(1000),
            &p,
            dec!(9000)
        ));
        assert!(Presupuesto::new(dec!(0), None).is_err());
        assert!(Presupuesto::new(dec!(100), Some(dec!(200))).is_err());
    }

    fn candidata(impresiones_hoy: u32) -> PiezaCandidata {
        PiezaCandidata {
            creativo: CreativoId::nuevo(),
            campana: CampanaId::nuevo(),
            anunciante_verificado: true,
            creativo_aprobado: true,
            estado_campana: EstadoCampana::Activa,
            inicio: ahora() - Duration::days(1),
            fin: ahora() + Duration::days(1),
            modelo: ModeloCobro::Cpc,
            tarifa: dec!(1000),
            presupuesto: Presupuesto::new(dec!(10000), None).expect("válido"),
            consumido_total: Decimal::ZERO,
            consumido_hoy: Decimal::ZERO,
            impresiones_hoy,
        }
    }

    #[test]
    fn solo_se_entregan_piezas_verificadas_aprobadas_vigentes_y_con_presupuesto() {
        let ok = candidata(0);
        assert!(ok.entregable(ahora()));
        let mut x = candidata(0);
        x.anunciante_verificado = false;
        assert!(!x.entregable(ahora()));
        let mut x = candidata(0);
        x.creativo_aprobado = false;
        assert!(!x.entregable(ahora()));
        let mut x = candidata(0);
        x.estado_campana = EstadoCampana::Pausada;
        assert!(!x.entregable(ahora()));
        let mut x = candidata(0);
        x.fin = ahora();
        assert!(!x.entregable(ahora()));
        let mut x = candidata(0);
        x.consumido_total = dec!(9500);
        assert!(!x.entregable(ahora()), "no alcanza ni para un clic");
    }

    #[test]
    fn la_seleccion_rota_y_da_una_pieza_por_campana() {
        let vista = candidata(10);
        let nueva = candidata(0);
        let mut misma_campana = candidata(1);
        misma_campana.campana = nueva.campana;
        let mut pausada = candidata(0);
        pausada.estado_campana = EstadoCampana::Pausada;
        let v = seleccionar_piezas(
            vec![vista.clone(), misma_campana, nueva.clone(), pausada],
            ahora(),
            5,
        );
        assert_eq!(v, [nueva, vista]);
    }

    #[test]
    fn frecuencia_de_un_masivo_por_semana() {
        assert!(puede_recibir_masivo(None, ahora()));
        assert!(!puede_recibir_masivo(
            Some(ahora() - Duration::days(6)),
            ahora()
        ));
        assert!(puede_recibir_masivo(
            Some(ahora() - Duration::days(7)),
            ahora()
        ));
    }

    fn contenido(formato: FormatoCreativo) -> ContenidoCreativo {
        ContenidoCreativo {
            formato,
            titulo: " 2x1 en arreglos ".into(),
            texto: "Toda la semana".into(),
            boton: None,
            enlace: None,
            condiciones: None,
            es_promocion: false,
            cupon: None,
            plantilla_whatsapp: None,
        }
    }

    #[test]
    fn las_promociones_declaran_condiciones() {
        assert_eq!(
            contenido(FormatoCreativo::Banner)
                .validado()
                .expect("válido")
                .titulo,
            "2x1 en arreglos"
        );
        let mut p = contenido(FormatoCreativo::Banner);
        p.es_promocion = true;
        assert!(p.clone().validado().is_err());
        p.condiciones = Some("Hasta el 30 de noviembre".into());
        assert!(p.validado().is_ok());
        let mut c = contenido(FormatoCreativo::OfertaCupon);
        c.cupon = Some(Cupon {
            codigo: CodigoCupon::new("rosa10").expect("válido"),
            limite_usos: Some(50),
        });
        assert!(c.clone().validado().is_err(), "el cupón exige condiciones");
        c.condiciones = Some("Un uso por compra".into());
        assert!(c.validado().expect("válido").es_promocion);
    }

    #[test]
    fn formatos_validan_lo_suyo() {
        let mut b = contenido(FormatoCreativo::Banner);
        b.enlace = Some("http://inseguro.co".into());
        assert!(b.validado().is_err());
        let mut b = contenido(FormatoCreativo::Banner);
        b.cupon = Some(Cupon {
            codigo: CodigoCupon::new("ABCD").expect("válido"),
            limite_usos: None,
        });
        assert!(b.validado().is_err(), "cupón solo en ofertas");
        let mut w = contenido(FormatoCreativo::MensajeWhatsapp);
        assert!(w.clone().validado().is_err(), "falta la plantilla");
        w.plantilla_whatsapp = Some("Oferta Semanal".into());
        assert!(w.clone().validado().is_err());
        w.plantilla_whatsapp = Some("oferta_semanal".into());
        assert!(w.validado().is_ok());
        assert_eq!(
            FormatoCreativo::MensajeWhatsapp.canal(),
            CanalPublicidad::Whatsapp
        );
        assert!(FormatoCreativo::BoletinCorreo.es_masivo());
        assert!(!FormatoCreativo::Banner.es_masivo());
    }

    #[test]
    fn cupones_con_limite_y_vigencia() {
        assert_eq!(
            CodigoCupon::new(" rosa10 ").map(|c| c.0),
            Ok("ROSA10".into())
        );
        assert!(CodigoCupon::new("ab").is_err());
        assert!(CodigoCupon::new("ROSA-10").is_err());
        let c = Cupon {
            codigo: CodigoCupon::new("ROSA10").expect("válido"),
            limite_usos: Some(2),
        };
        assert!(validar_redencion(&c, 1, true).is_ok());
        assert!(validar_redencion(&c, 2, true).is_err());
        assert!(validar_redencion(&c, 0, false).is_err());
    }

    #[test]
    fn la_liquidacion_suma_variable_fijos_y_leads() {
        assert_eq!(
            valor_liquidacion(dec!(1234.567), dec!(90000), 3, dec!(5000)),
            dec!(106234.57)
        );
    }
}
