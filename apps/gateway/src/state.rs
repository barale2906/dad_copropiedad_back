use std::sync::Arc;

use infra_postgres::PgPool;
use shared::WhatsappConfig;

#[derive(Clone)]
pub struct GatewayState {
    pub db: PgPool,
    pub whatsapp: Arc<WhatsappConfig>,
}
