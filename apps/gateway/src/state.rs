use std::sync::Arc;

use infra_archivos::AlmacenLocal;
use infra_postgres::{
    DirectorioAsambleasPg, DirectorioTelefonosPg, PgPool, SesionesChatPg, UnidadDeTrabajoPg,
};
use shared::WhatsappConfig;
use tokio::sync::Notify;

use crate::mensajeria::MensajeriaGateway;

/// Estado compartido del gateway. Clonarlo es barato (pools y `Arc`).
#[derive(Clone)]
pub struct GatewayState {
    pub db: PgPool,
    pub whatsapp: Arc<WhatsappConfig>,
    /// Transacciones de copropiedad (RLS por tenant) para los casos de uso del chat.
    pub unidad: UnidadDeTrabajoPg,
    pub sesiones: SesionesChatPg,
    pub directorio: DirectorioTelefonosPg,
    /// Asistencias por teléfono y acuses de entrega (asambleas, cruza copropiedades).
    pub asambleas: DirectorioAsambleasPg,
    pub mensajeria: MensajeriaGateway,
    pub archivos: Arc<AlmacenLocal>,
    /// Despierta al worker de salida apenas el chat encola respuestas.
    pub hay_salida: Arc<Notify>,
    /// Firma de los QR de visitas que se autorizan desde el chat (`ACCESO_QR_SECRETO`).
    pub firma_qr: Arc<infra_seguridad::FirmaQrEd25519>,
}
