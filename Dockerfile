# syntax=docker/dockerfile:1.7
# Imagen de producción de DAD_COPROPIEDAD (AGENTS.md §10): multi-stage, imagen final mínima,
# usuario no root, sin herramientas de compilación. Contiene la API, el gateway y dad_admin
# (migraciones y superusuario); el comando se elige al ejecutar.
#
#   docker build -t dad_copropiedad:local .
#   docker run --env-file .env.produccion dad_copropiedad:local dad_copropiedad_api
#
# El puente de acceso (apps/puente-acceso) no va aquí: corre en el mini PC de cada PH.

# --- Compilación (versión fijada en rust-toolchain.toml) ---
FROM rust:1.99-slim-bookworm AS compilacion
WORKDIR /fuente
ENV SQLX_OFFLINE=true \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/fuente/target \
    cargo build --release --locked -p api -p gateway --bins \
 && mkdir -p /salida \
 && cp target/release/dad_copropiedad_api target/release/dad_copropiedad_gateway \
       target/release/dad_admin /salida/

# --- Prueba de carga (solo con `docker build --target carga`; no va en la imagen de producción) ---
FROM compilacion AS compilacion-carga
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/fuente/target \
    cargo build --release --locked -p carga --bins \
 && cp target/release/dad_carga /salida/

FROM debian:bookworm-slim AS carga
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /carga --create-home --shell /usr/sbin/nologin dad
COPY --from=compilacion-carga /salida/dad_carga /usr/local/bin/
WORKDIR /carga
USER dad
ENTRYPOINT ["/usr/local/bin/dad_carga"]

# --- Ejecución (etapa final: la imagen por defecto) ---
FROM debian:bookworm-slim AS ejecucion
# TLS con rustls y raíces de webpki embebidas: no hace falta OpenSSL. `ca-certificates` solo por
# si un proveedor SMTP usa una autoridad local; `tini` reenvía SIGTERM (apagado ordenado).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin dad \
 && mkdir -p /app/datos/archivos \
 && chown -R dad:dad /app
COPY --from=compilacion /salida/ /usr/local/bin/
WORKDIR /app
USER dad
ENV LOG_FORMAT=json \
    ARCHIVOS_DIR=/app/datos/archivos \
    APP_HOST=0.0.0.0
VOLUME ["/app/datos"]
EXPOSE 8080 8081
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dad_copropiedad_api"]
