//! Estado compartido de la API: adaptadores ya construidos que reciben los handlers.

use std::sync::Arc;

use application::casos_uso::facturacion::documentos::Expedicion;

use infra_archivos::AlmacenLocal;
use infra_facturacion::{
    EmpaquetadorZip, LectorPkcs12, LectorUbl, MotorPropio, ProveedorDian, ProveedorSimulado,
};
use infra_pdf::GeneradorTypst;
use infra_planillas::LectorCsvXlsx;
use infra_postgres::{
    DirectorioAccesoPg, IdempotenciaPg, PgPool, UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg,
};
use infra_seguridad::{Argon2Contrasenas, CofreAesGcm, FirmaQrEd25519, Totp};
use shared::config::{DIAN_URL_HABILITACION, DIAN_URL_PRODUCCION};

/// Estado compartido entre handlers. Clonarlo es barato (todo va detrás de `Arc` o es un pool).
///
/// Aquí se inyectan los adaptadores que consumen los casos de uso de `application`.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Transacciones de copropiedad (RLS por tenant).
    pub unidad: UnidadDeTrabajoPg,
    /// Transacciones de plataforma (cuentas, sesiones, roles de plataforma, organizaciones).
    pub plataforma: UnidadDeTrabajoPlataformaPg,
    /// Almacén de archivos configurado (`ARCHIVOS_ALMACEN`). Hoy solo el disco local.
    pub archivos: Arc<AlmacenLocal>,
    pub contrasenas: Argon2Contrasenas,
    pub totp: Arc<Totp>,
    /// Lector de planillas CSV/XLSX de las importaciones masivas.
    pub planillas: LectorCsvXlsx,
    /// `PLATAFORMA_EXIGE_SEGUNDO_FACTOR`.
    pub plataforma_exige_segundo_factor: bool,
    /// `REGISTRO_ABIERTO`: `new` lo deja **abierto** (las pruebas crean cuentas así); `main` lo
    /// configura (cerrado por defecto).
    pub registro_abierto: bool,
    /// Actas y hojas de rótulos en PDF (Typst).
    pub actas: GeneradorTypst,
    /// Tableros en vivo de las asambleas (WebSocket).
    pub en_vivo: Arc<crate::en_vivo::EnVivo>,
    /// Firma Ed25519 de los QR de visitas (clave derivada por PH de `ACCESO_QR_SECRETO`).
    pub firma_qr: Arc<FirmaQrEd25519>,
    /// Autenticación de los puentes de acceso (función `SECURITY DEFINER`).
    pub acceso: DirectorioAccesoPg,
    /// Límite de peticiones por IP. `new` lo crea **sin límites**; `main` lo configura con
    /// `LIMITE_*` (las pruebas que lo verifican lo reemplazan).
    pub limitador: Arc<crate::limite::Limitador>,
    /// Registro de `Idempotency-Key` (reintentos seguros de los POST).
    pub idempotencia: IdempotenciaPg,
    /// Proveedor de facturación electrónica: el motor propio para las entidades con el canal
    /// `propio` (llevan sus credenciales) y el simulado para las demás (ADR 0020 y 0021).
    pub facturacion: ProveedorDian,
    /// Cofre de los secretos de facturación de cada entidad (`FACTURACION_CLAVE_MAESTRA`).
    /// `new` lo crea **sin clave**; `main` lo configura.
    pub cofre: Arc<CofreAesGcm>,
    /// Lector de los certificados `.p12` que cargan las entidades.
    pub lector_certificados: LectorPkcs12,
    /// Pie de la representación gráfica (software y fabricante, `FACTURACION_*`).
    pub pie_facturacion: Option<String>,
    /// Lector de las facturas electrónicas recibidas (XML UBL).
    pub lector_facturas: LectorUbl,
    /// ZIP del paquete de documentos del periodo para el contador.
    pub empaquetador: EmpaquetadorZip,
}

impl AppState {
    /// Lo que usa la expedición de documentos electrónicos.
    pub fn expedicion(
        &self,
    ) -> Expedicion<'_, ProveedorDian, CofreAesGcm, GeneradorTypst, EmpaquetadorZip> {
        Expedicion {
            proveedor: &self.facturacion,
            cofre: self.cofre.as_ref(),
            generador: &self.actas,
            empaquetador: &self.empaquetador,
            pie: self.pie_facturacion.as_deref(),
        }
    }

    /// Estado con los adaptadores reales sobre `db`.
    pub fn new(
        db: PgPool,
        archivos: AlmacenLocal,
        plataforma_exige_segundo_factor: bool,
        firma_qr: FirmaQrEd25519,
    ) -> Self {
        Self {
            unidad: UnidadDeTrabajoPg::new(db.clone()),
            plataforma: UnidadDeTrabajoPlataformaPg::new(db.clone()),
            acceso: DirectorioAccesoPg::new(db.clone()),
            idempotencia: IdempotenciaPg::new(db.clone()),
            db,
            archivos: Arc::new(archivos),
            contrasenas: Argon2Contrasenas,
            totp: Arc::new(Totp::new("DAD Copropiedad")),
            planillas: LectorCsvXlsx,
            plataforma_exige_segundo_factor,
            registro_abierto: true,
            actas: GeneradorTypst,
            en_vivo: Arc::new(crate::en_vivo::EnVivo::new()),
            firma_qr: Arc::new(firma_qr),
            limitador: Arc::new(crate::limite::Limitador::new(
                crate::limite::Limites::ninguno(),
            )),
            facturacion: ProveedorDian::new(
                ProveedorSimulado::new(),
                MotorPropio::new(DIAN_URL_HABILITACION, DIAN_URL_PRODUCCION)
                    .expect("el cliente TLS de la DIAN se arma con las raíces de webpki"),
            ),
            cofre: Arc::new(CofreAesGcm::new(None)),
            lector_certificados: LectorPkcs12,
            pie_facturacion: None,
            lector_facturas: LectorUbl,
            empaquetador: EmpaquetadorZip,
        }
    }
}
