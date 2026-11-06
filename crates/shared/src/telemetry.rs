//! Inicialización de logs estructurados con `tracing`.

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::LogFormat;

/// Filtro por defecto si `RUST_LOG` no está definida.
const DEFAULT_FILTER: &str = "info,dad_copropiedad_api=debug,dad_copropiedad_gateway=debug,\
     domain=debug,application=debug,infra_postgres=debug,infra_whatsapp=debug,\
     tower_http=info,sqlx=warn";

/// Configura el suscriptor global de `tracing`. Debe llamarse una sola vez al iniciar.
pub fn init(format: LogFormat) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let registry = tracing_subscriber::registry().with(filter);

    match format {
        LogFormat::Pretty => registry.with(fmt::layer().with_target(true)).init(),
        LogFormat::Json => registry
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .flatten_event(true),
            )
            .init(),
    }
}
