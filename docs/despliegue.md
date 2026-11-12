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

## 9. Rendimiento y prueba de carga

### Qué determina la velocidad

Docker no es el límite: los binarios corren nativos y los volúmenes escriben directo al disco.
Lo que pesa, con varias copropiedades en el mismo servidor, es lo que todas comparten:

1. **PostgreSQL**: `docker-compose.prod.yml` lo ajusta a la RAM con las variables `PG_*`
   (de fábrica usa 128 MB de caché, pensado para una máquina mínima).
2. **Pools de conexiones**: `API_DB_CONEXIONES` y `GATEWAY_DB_CONEXIONES` se reparten entre
   **todas** las PH; con varias asambleas a la vez, un pool pequeño hace esperar turno.
3. **Memoria compartida con HestiaCP**: Nginx, PHP, correo y, si están activos, ClamAV y
   SpamAssassin consumen RAM. Los `LIMITE_MEMORIA_*` evitan que un pico de uno le quite memoria
   a PostgreSQL.

### Valores por tamaño del VPS

Suponen HestiaCP en la misma máquina (reserva ~1,5 GB para el panel y sus servicios). Se
definen en `.env.produccion`; lo que no se defina toma la columna de 4 GB.

| Variable | 4 GB | 8 GB | 16 GB |
|---|---|---|---|
| `PG_SHARED_BUFFERS` | 512MB | 1GB | 2GB |
| `PG_EFFECTIVE_CACHE_SIZE` | 1536MB | 3GB | 7GB |
| `PG_WORK_MEM` | 8MB | 16MB | 16MB |
| `PG_MAINTENANCE_WORK_MEM` | 128MB | 256MB | 512MB |
| `PG_MAX_CONNECTIONS` | 60 | 80 | 120 |
| `API_DB_CONEXIONES` | 30 | 40 | 60 |
| `GATEWAY_DB_CONEXIONES` | 15 | 20 | 30 |
| `LIMITE_MEMORIA_POSTGRES` | 1536m | 3g | 6g |
| `LIMITE_MEMORIA_API` / `_GATEWAY` | 512m | 768m | 1g |

`PG_MAX_CONNECTIONS` debe ser mayor que la suma de los dos pools más un margen para
`dad_admin`, `psql` y los respaldos. `PG_LOG_CONSULTAS_LENTAS_MS` (500 por defecto) deja en el
log de PostgreSQL las consultas lentas: es lo primero que se revisa si una operación se degrada.
Los valores son un punto de partida: los confirma la prueba de carga.

Ajuste opcional del host: con `"userland-proxy": false` en `/etc/docker/daemon.json`, Docker
deja de pasar el tráfico Nginx → API por un proceso intermedio (ahorra decenas de microsegundos
por petición). Exige reiniciar Docker: hágalo en una ventana de mantenimiento.

### Prueba de carga (`dad_carga`)

Mide cuántas copropiedades aguanta el servidor operando a la vez: en cada una, varias sesiones
navegan (capacidades, bienes, tablero y quórum de la asamblea) mientras **todas** las asambleas
votan al mismo tiempo. Informa p50, p95, p99, máximo, peticiones por segundo y errores por
operación, y falla si se registran menos votos que asistentes o si los errores superan el tope.

**Nunca contra producción**: la siembra crea copropiedades, cuentas y votos que no se pueden
borrar (la auditoría y los votos son append-only). Se usa un entorno de prueba aislado, con
la misma configuración, en un VPS igual o en el mismo VPS **antes de salir a producción** (o en
una ventana de mantenimiento: la prueba compite por los mismos recursos).

```bash
# 1. Local: la herramienta en una imagen con la misma base que producción
docker build --target carga -t dad_carga .
docker save dad_carga | ssh usuario@vps docker load

# 2. VPS: entorno de prueba aparte (otro proyecto, otros volúmenes, otros puertos). Sin gateway:
#    no se envía nada. .env.carga es una copia de .env.produccion con API_PUERTO=18080,
#    GATEWAY_PUERTO=18081 y LIMITE_AUTH_POR_MINUTO=0.
alias dcc='docker compose -p dad_carga -f docker-compose.prod.yml --env-file .env.carga'
dcc up -d postgres && dcc run --rm migracion && dcc up -d api
read -rs -p "Contraseña del superusuario de prueba: " CARGA_ROOT_CONTRASENA; echo
export CARGA_ROOT_CONTRASENA CARGA_ROOT_USUARIO=carga_root CARGA_CONTRASENA="$CARGA_ROOT_CONTRASENA"
DAD_ADMIN_CONTRASENA="$CARGA_ROOT_CONTRASENA" dcc run --rm -e DAD_ADMIN_CONTRASENA api \
  dad_admin crear-superusuario --usuario carga_root --nombre "Prueba de carga"

# 3. Sembrar (una vez) y ejecutar (las veces que quiera)
mkdir -p ~/carga && chmod 777 ~/carga
correr() { docker run --rm --network host -v ~/carga:/carga \
  -e CARGA_CONTRASENA -e CARGA_ROOT_USUARIO -e CARGA_ROOT_CONTRASENA -e CARGA_ROOT_TOTP dad_carga "$@"; }
correr sembrar --api http://127.0.0.1:18080 --copropiedades 20 --bienes 300 --plan plan.json
correr ejecutar --api http://127.0.0.1:18080 --plan plan.json \
  --duracion 120 --usuarios 5 --concurrencia-votos 10 --salida informe.json

# 4. Al terminar: borrar el entorno de prueba con sus volúmenes
dcc down -v
```

Parámetros de `ejecutar`: `--duracion` (s), `--usuarios` (sesiones navegando por PH),
`--pausa-ms` (pausa de cada sesión entre peticiones; 300), `--concurrencia-votos` (puestos de
la mesa votando a la vez por PH), `--inicio-votacion` (s) y `--max-errores-por-mil` (10).
Cada ejecución abre su propia pregunta y la cierra al terminar, así que se puede repetir sobre
la misma siembra variando la carga.

**Cómo leer el resultado.** Suba el número de copropiedades o de usuarios hasta que el p95 de
`votar` o de `tablero` supere lo aceptable (orientativo: 500 ms) o aparezcan errores; esa es la
capacidad del servidor con esa configuración. Si la CPU de PostgreSQL llega al tope, ajuste
`PG_*` o aumente núcleos; si las peticiones esperan con la base ociosa, suba los pools.
