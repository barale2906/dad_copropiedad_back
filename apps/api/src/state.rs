use std::sync::Arc;

use infra_archivos::AlmacenLocal;
use infra_pdf::GeneradorTypst;
use infra_planillas::LectorCsvXlsx;
use infra_postgres::{DirectorioAccesoPg, PgPool, UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use infra_seguridad::{Argon2Contrasenas, FirmaQrEd25519, Totp};

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
    /// Actas y hojas de rótulos en PDF (Typst).
    pub actas: GeneradorTypst,
    /// Tableros en vivo de las asambleas (WebSocket).
    pub en_vivo: Arc<crate::en_vivo::EnVivo>,
    /// Firma Ed25519 de los QR de visitas (clave derivada por PH de `ACCESO_QR_SECRETO`).
    pub firma_qr: Arc<FirmaQrEd25519>,
    /// Autenticación de los puentes de acceso (función `SECURITY DEFINER`).
    pub acceso: DirectorioAccesoPg,
}

impl AppState {
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
            db,
            archivos: Arc::new(archivos),
            contrasenas: Argon2Contrasenas,
            totp: Arc::new(Totp::new("DAD Copropiedad")),
            planillas: LectorCsvXlsx,
            plataforma_exige_segundo_factor,
            actas: GeneradorTypst,
            en_vivo: Arc::new(crate::en_vivo::EnVivo::new()),
            firma_qr: Arc::new(firma_qr),
        }
    }
}
