# Despliegue en el VPS

Guía de operación de producción: VPS Ubuntu con HestiaCP (Nginx con TLS), contenedores con
Docker Compose y la imagen que publica el CI. Complementa `AGENTS.md` §10.

## 1. Piezas

| Pieza | Dónde | Notas |
|---|---|---|
| Imagen `ghcr.io/barale2906/dad_copropiedad_back:sha-…` | GHCR | La publica el job `imagen` del CI en cada push a `main`. Etiqueta fija por commit; nunca `latest`. |
| `api` (`dad_copropiedad_api`) | `127.0.0.1:8080` | REST, WebSocket del tablero en vivo, `/health`, `/ready`. |
| `gateway` (`dad_copropiedad_gateway`) | `127.0.0.1:8081` | Webhook de WhatsApp, outbox (correo, WhatsApp, push) y workers. |
| `migracion` (`dad_admin migrar`) | una vez por despliegue | Aplica migraciones con el dueño del esquema y termina. |
| `postgres` | red interna de Compose | PostgreSQL 18; nunca se publica a internet. |
| Volumen `archivos` | `/app/datos/archivos` | Lo comparten la API y el gateway (fotos, firmas, actas). **Va en el respaldo.** |
| Puente de acceso | mini PC de cada PH | Binario aparte (`cargo build --release -p puente-acceso`), ver README. |

## 2. Variables (`.env.produccion`, fuera de git, permisos 600)

Parten de `.env.example`. Las de producción que no pueden faltar:

- `APP_DATABASE_URL=postgres://dad_app:…@postgres:5432/dad_copropiedad` (rol restringido) y
  `DATABASE_URL=postgres://<dueño>:…@postgres:5432/dad_copropiedad` (solo lo usa `migracion`).
- `POSTGRES_USER`, `POSTGRES_PASSWORD`, `APP_DB_PASSWORD` (crean la BD y el rol `dad_app` la
  primera vez; `docker/postgres/init`).
- `LOG_FORMAT=json`, `API_DOCS_HABILITADA=false`, `PLATAFORMA_EXIGE_SEGUNDO_FACTOR=true`,
  `RUN_MIGRATIONS=false`.
- Secretos generados con `openssl rand -hex 32`: `ARCHIVOS_SECRETO_ENLACES`,
  `ACCESO_QR_SECRETO`. Rotarlos invalida enlaces de descarga y QR de visitas vigentes.
- `ARCHIVOS_URL_PUBLICA=https://api.<dominio>`.
- Límites: `CONFIAR_PROXY=true` (Nginx fija `X-Real-IP`), `LIMITE_AUTH_POR_MINUTO`,
  `LIMITE_PUBLICO_POR_MINUTO`.
- `SMTP_*` (HestiaCP), `WHATSAPP_*` (Meta), `FCM_CREDENCIALES=/app/secretos/firebase.json`
  (montar el JSON como volumen de solo lectura en `api` y `gateway` si se usa push).

## 3. Primer despliegue

```bash
echo "$GHCR_TOKEN" | docker login ghcr.io -u <usuario> --password-stdin
export IMAGEN=ghcr.io/barale2906/dad_copropiedad_back:sha-<commit>
docker compose -f docker-compose.prod.yml --env-file .env.produccion up -d postgres
docker compose -f docker-compose.prod.yml --env-file .env.produccion run --rm migracion
docker compose -f docker-compose.prod.yml --env-file .env.produccion up -d api gateway
DAD_ADMIN_CONTRASENA='…' docker compose -f docker-compose.prod.yml --env-file .env.produccion \
  run --rm -e DAD_ADMIN_CONTRASENA api dad_admin crear-superusuario --usuario admin --correo admin@<dominio>
curl -fsS http://127.0.0.1:8080/ready
```

## 4. Actualizar a una versión nueva

1. Respaldo de la BD (§6).
2. `export IMAGEN=…:sha-<nuevo>` y `docker compose … pull`.
3. `docker compose … run --rm migracion` (las migraciones son reversibles; nunca se editan las
   aplicadas).
4. `docker compose … up -d api gateway` (apagado ordenado con SIGTERM vía `tini`).
5. Verificar `/ready` y los logs (`docker compose … logs -f --since 5m`).

Volver atrás: `IMAGEN` anterior + `up -d`. Si la migración nueva rompe, revertirla con
`sqlx migrate revert` desde una máquina con acceso y el dueño del esquema, antes de bajar la
imagen.

## 5. Nginx (HestiaCP)

Plantilla personalizada para `api.<dominio>` (TLS con Let's Encrypt de HestiaCP):

```nginx
location / {
    proxy_pass         http://127.0.0.1:8080;
    proxy_set_header   Host $host;
    proxy_set_header   X-Real-IP $remote_addr;          # el limitador confía en esta cabecera
    proxy_set_header   X-Forwarded-Proto $scheme;
    proxy_set_header   X-Request-Id $request_id;
    client_max_body_size 30m;                            # fotos en Base64 de la portería
}
# WebSocket del tablero en vivo de las asambleas
location ~ ^/api/v1/copropiedades/[^/]+/asambleas/[^/]+/en-vivo$ {
    proxy_pass         http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header   Upgrade $http_upgrade;
    proxy_set_header   Connection "upgrade";
    proxy_set_header   X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
}
```

`wa.<dominio>` (webhook de Meta) apunta a `127.0.0.1:8081` con el mismo bloque `location /`.
Nginx debe **reemplazar** `X-Real-IP` (no pasar la del cliente): por eso `CONFIAR_PROXY=true`
solo es seguro detrás de este proxy. Para varias instancias de la API, agregar `limit_req` en
Nginx: el limitador en memoria es por proceso.

## 6. Respaldos

- BD: `docker compose … exec -T postgres pg_dump -U $POSTGRES_USER -Fc dad_copropiedad >
  respaldo-$(date +%F).dump` diario, con retención y copia fuera del VPS. Probar la restauración
  (`pg_restore`) al menos una vez al trimestre.
- Archivos: el volumen `archivos` (o `ARCHIVOS_DIR`) con la misma frecuencia.
- `.env.produccion` y la cuenta de servicio de Firebase, en el gestor de secretos del equipo.

## 7. Verificación después de cada despliegue

- `GET /health` 200 y `GET /ready` 200 (BD disponible).
- Logs JSON sin errores de arranque; el gateway informa los canales activos (correo, WhatsApp,
  push) y los workers iniciados.
- Un inicio de sesión de prueba y, si se tocó el módulo, una visita anunciada de prueba.
