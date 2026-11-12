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

## 3. Preparar el VPS (una sola vez)

El VPS no compila nada ni necesita el código fuente: ejecuta la imagen que publicó el CI.
Solo necesita Docker y tres cosas de este repositorio.

```bash
# En el VPS
sudo apt-get update && sudo apt-get install -y ca-certificates curl
curl -fsSL https://get.docker.com | sudo sh        # Docker Engine + plugin compose
sudo usermod -aG docker "$USER"                     # cerrar sesión y volver a entrar
sudo mkdir -p /opt/dad && sudo chown "$USER" /opt/dad

# Desde la máquina local (raíz del repositorio)
scp docker-compose.prod.yml usuario@vps:/opt/dad/
scp -r docker usuario@vps:/opt/dad/                  # scripts de inicio de PostgreSQL
scp .env.produccion usuario@vps:/opt/dad/            # secretos: nunca en git (§2)
ssh usuario@vps 'chmod 600 /opt/dad/.env.produccion'
```

Para descargar imágenes de GHCR (paquete privado) cree un token de GitHub con permiso
`read:packages` e inicie sesión una vez en el VPS:
`echo "$GHCR_TOKEN" | docker login ghcr.io -u <usuario> --password-stdin`.

Los comandos siguientes se ejecutan en `/opt/dad`. Para abreviar:
`alias dc='docker compose -f docker-compose.prod.yml --env-file .env.produccion'`.

## 4. Primer despliegue

```bash
export IMAGEN=ghcr.io/barale2906/dad_copropiedad_back:sha-<commit>   # la del CI, nunca latest
dc up -d postgres                 # crea la BD y el rol dad_app la primera vez
dc run --rm migracion             # dad_admin migrar
dc up -d api gateway
curl -fsS http://127.0.0.1:8080/ready
```

**Primer superusuario.** La contraseña se lee sin mostrarla y sin dejarla en el historial del
shell (mínimo 10 caracteres):

```bash
read -rs -p "Contraseña del superusuario: " DAD_ADMIN_CONTRASENA; echo
export DAD_ADMIN_CONTRASENA
dc run --rm -e DAD_ADMIN_CONTRASENA api \
  dad_admin crear-superusuario --usuario <usuario> --correo <correo> --nombre "<Nombre>"
unset DAD_ADMIN_CONTRASENA
```

Si la cuenta ya existe, solo recibe el rol. Después, inicie sesión en la web y active el
segundo factor: la consola de plataforma lo exige.

## 5. Actualizar a una versión nueva

1. Respaldo de la BD y de los archivos (§7).
2. `export IMAGEN=…:sha-<nuevo>` y `dc pull`.
3. `dc run --rm migracion` (las migraciones son reversibles; nunca se editan las aplicadas).
4. `dc up -d api gateway` (apagado ordenado con SIGTERM vía `tini`).
5. Verificar `/ready` y los logs (`dc logs -f --since 5m api gateway`).

**Volver atrás.** Con una imagen anterior que no conoce las migraciones nuevas, primero se
revierten esas migraciones **con la imagen nueva** (es la que tiene sus `.down.sql`) y después
se baja la imagen:

```bash
dc exec -T postgres psql -U "$POSTGRES_USER" -d dad_copropiedad \
  -c "SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 5"
dc run --rm migracion dad_admin revertir --hasta <última versión que debe quedar> --confirmar
export IMAGEN=…:sha-<anterior> && dc up -d api gateway
```

`revertir` es destructivo (un `.down.sql` puede borrar tablas o columnas con datos): exige la
versión destino y `--confirmar`, y va siempre después del respaldo. Si la versión anterior no
cambió el esquema, basta con bajar la imagen.

## 6. Nginx (HestiaCP)

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

## 7. Base de datos y respaldos

PostgreSQL es el contenedor `postgres`; sus datos viven en el volumen `pg_data` y nunca se
publica a internet (solo lo ven la API, el gateway y `migracion` por la red interna).

- Consola SQL: `dc exec postgres psql -U "$POSTGRES_USER" -d dad_copropiedad`. Para consultar
  datos de negocio como lo hace la app, `SET ROLE dad_app;` y `SET app.tenant_id = '<uuid>';`.
- Herramienta gráfica desde su equipo: túnel SSH, nunca abrir el puerto. Requiere publicar
  PostgreSQL solo en la interfaz local del VPS (`ports: ["127.0.0.1:5432:5432"]` en el
  servicio `postgres`) y luego `ssh -L 5432:127.0.0.1:5432 usuario@vps`.
- BD: `dc exec -T postgres pg_dump -U "$POSTGRES_USER" -Fc dad_copropiedad >
  respaldo-$(date +%F).dump` diario (cron), con retención y copia fuera del VPS. Restaurar:
  `dc exec -T postgres pg_restore -U "$POSTGRES_USER" -d dad_copropiedad --clean < archivo.dump`,
  probado al menos una vez al trimestre.
- Archivos: el volumen `archivos` (o `ARCHIVOS_DIR`) con la misma frecuencia.
- `.env.produccion` y la cuenta de servicio de Firebase, en el gestor de secretos del equipo.

## 8. Verificación después de cada despliegue

- `GET /health` 200 y `GET /ready` 200 (BD disponible).
- Logs JSON sin errores de arranque; el gateway informa los canales activos (correo, WhatsApp,
  push) y los workers iniciados.
- Un inicio de sesión de prueba y, si se tocó el módulo, una visita anunciada de prueba.
