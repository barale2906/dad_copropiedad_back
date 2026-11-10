# dad_copropiedad_back

[![CI](https://github.com/barale2906/dad_copropiedad_back/actions/workflows/ci.yml/badge.svg)](https://github.com/barale2906/dad_copropiedad_back/actions/workflows/ci.yml)

Backend de **DAD_COPROPIEDAD**, el software de gestión de propiedad horizontal de DAD AI & SYSTEMS.
El primer módulo es **votaciones** en asambleas: multi-PH, con votos por web y por WhatsApp (botones o listas).

El backend es un monolito modular con arquitectura hexagonal, hecho con Rust, Axum, SQLx y PostgreSQL (más Typst para las actas en PDF).

## Requisitos
- Rust (rustup; la versión se instala sola según `rust-toolchain.toml`)
- Docker + Docker Compose
- `sqlx-cli`, `cargo-nextest`, `bacon` (opcionales: `cargo-deny`, `cargo-audit`, `cargo-llvm-cov`)

## Arranque rápido
Con Make (`make` lista todos los comandos):
```bash
make herramientas   # una vez: sqlx-cli, nextest, bacon, deny, audit, llvm-cov
make setup          # .env + contenedores + migraciones
make api            # API en :8080   ·   make gateway → :8081
make verificar      # fmt + clippy + pruebas, antes de terminar una tarea
```

Equivalente sin Make:
```bash
cp .env.example .env          # variables locales
docker compose up -d --wait   # PostgreSQL 18 (+ rol dad_app) y Mailpit
cargo api                     # aplica migraciones y levanta la API en :8080
cargo gateway                 # webhook de WhatsApp en :8081

curl localhost:8080/health    # {"status":"ok",...}
curl localhost:8080/ready     # {"database":"ok","status":"ok"}
```
Correos en desarrollo: `docker compose up -d` también levanta **Mailpit**; todo lo que envíe el
gateway aparece en <http://localhost:8025> (no sale a internet).

WhatsApp: el gateway recibe el webhook en `/webhook/whatsapp` y atiende el chat de la
copropiedad (menú, reportes con foto, reservas, bandeja humana). Para **enviar** necesita
`WHATSAPP_ACCESS_TOKEN`; sin él recibe y deja las respuestas pendientes en el outbox. Las
plantillas `ticket_actualizado` y `reporte_urgente` deben estar aprobadas en Meta (ADR 0009). El
gateway usa las mismas variables `ARCHIVOS_*` de la API para guardar las fotos recibidas, y su
worker de gestión cierra tickets resueltos, alerta los vencidos y genera las órdenes preventivas
cada 10 minutos.

Asambleas (plan 06, ADR 0010): el gateway atiende primero los mensajes de una asamblea
(botón «Confirmar» de la plantilla y votos con botones cuyo id lleva la pregunta) y luego el
chat; registra los acuses de entrega (`statuses`) y la calidad de las líneas, y su worker de
asambleas cierra cada 5 s las preguntas con temporizador vencido y las consultas por escrito.
Deben estar aprobadas en Meta las plantillas de utilidad `confirmar_asistencia` (con un botón de
respuesta rápida «Confirmar») y `convocatoria_asamblea` (4 parámetros; en consultas por escrito,
con un botón «Votar»). La API emite el tablero en vivo por WebSocket
(`/api/v1/copropiedades/{id}/asambleas/{reunion}/en-vivo`, token como subprotocolo
`["dad.v1", token]`), recalculado desde la BD a lo sumo cada 500 ms tras cada `pg_notify`; no
necesita Redis. Las líneas de WhatsApp y las tarifas de Meta se gestionan en
`/api/v1/plataforma/whatsapp/…`.

Control de acceso (plan 05, ADR 0011, módulo `control_acceso`): la portería anuncia visitas y
registra correspondencia en `/api/v1/copropiedades/{id}/visitas` y `/correspondencia`. Los avisos
van **por la app (push) si el residente la tiene** y si no por WhatsApp; el worker de acceso del
gateway (cada 15 s) escala las visitas a los 2 y 5 minutos y repite los avisos de correspondencia
cada 24 h fuera del horario de silencio. La API necesita `ACCESO_QR_SECRETO` (≥ 32 caracteres)
para firmar los QR de las visitas pre-autorizadas; el gateway envía push si `FCM_CREDENCIALES`
apunta al JSON de la cuenta de servicio de Firebase. Deben aprobarse en Meta las plantillas
`visita_anunciada` (botones «Autorizar» y «Rechazar»), `correspondencia_recibida` y
`correspondencia_entregada`. El puente en sitio se prueba con el controlador simulado:

```bash
# Registre el puente en POST /api/v1/copropiedades/{id}/puentes y use el token que devuelve.
PUENTE_API_URL=http://localhost:8080/api/v1 PUENTE_TOKEN=<token> cargo puente
# Cada línea de la entrada estándar es una lectura: <punto> <entrada|salida> <tipo|qr> <valor>
```

Primer acceso: cree el superusuario con `make superusuario USUARIO=admin CORREO=admin@dominio.co`
(pide la contraseña), inicie sesión en `POST /api/v1/auth/sesion` desde `/docs` y use el token
con el botón *Authorize*. Con él se dan de alta copropiedades (`POST /api/v1/plataforma/copropiedades`)
y se asignan sus administradores. En desarrollo `PLATAFORMA_EXIGE_SEGUNDO_FACTOR=false`; en
producción la plataforma exige segundo factor (`/api/v1/yo/segundo-factor`).

Archivos en desarrollo: se guardan en `./datos/archivos` (`ARCHIVOS_DIR`, ignorado por git). En
producción, un directorio fuera del código incluido en la copia de seguridad del VPS y un
`ARCHIVOS_SECRETO_ENLACES` propio (`openssl rand -hex 32`). Migrar luego a S3: ADR 0003.

Con `API_DOCS_HABILITADA=true` (así viene en `.env.example`): documentación interactiva en
<http://localhost:8080/docs> y contrato en <http://localhost:8080/api-docs/openapi.json>.

> El rol `dad_app` se crea con login la **primera vez** que se crea el volumen de PostgreSQL
> (`docker/postgres/init`). Si ya tenías el volumen: `docker compose down -v && docker compose up -d --wait`.

## Producción

Despliegue en el VPS con la imagen que publica el CI (GHCR), `docker-compose.prod.yml` y Nginx de
HestiaCP: ver [`docs/despliegue.md`](docs/despliegue.md). Las migraciones son un paso explícito
(`dad_admin migrar`). Detrás de Nginx: `CONFIAR_PROXY=true` para que el límite de peticiones use
la IP real (`LIMITE_AUTH_POR_MINUTO`, `LIMITE_PUBLICO_POR_MINUTO`).

Protección de datos (Ley 1581, ADR 0004): la PH publica su política en
`/api/v1/copropiedades/{id}/proteccion-datos/politicas`; cada titular autoriza finalidades,
consulta «mis datos» y radica consultas o reclamos, que la administración tramita, prorroga una
vez y responde dentro de los plazos legales. La auditoría se consulta en `/auditoria`
(`auditoria.ver`).

## Desarrollo
```bash
bacon                 # revisión continua al guardar
cargo pruebas         # pruebas (nextest; las de esquema usan PostgreSQL)
cargo lint            # clippy estricto
cargo fmt --all       # formato
cargo cov             # cobertura HTML
cargo deny check      # licencias, fuentes y vulnerabilidades
```

Migraciones (se ejecutan con `DATABASE_URL`, el dueño del esquema):
```bash
sqlx migrate add -r <nombre>   # nueva migración reversible
sqlx migrate run               # aplicar
sqlx migrate revert            # revertir la última
```

## Estructura
| Ruta | Capa | Contenido |
|---|---|---|
| `crates/domain` | Dominio | Entidades, estados, quórum, coeficientes. Rust puro, sin IO |
| `crates/application` | Aplicación | Casos de uso y puertos (traits) |
| `crates/infra-postgres` | Adaptador | SQLx, RLS por tenant, repositorios |
| `crates/infra-whatsapp` | Adaptador | Cloud API de Meta: firma, webhook, envío |
| `crates/infra-pdf` | Adaptador | Actas con Typst (pendiente) |
| `crates/infra-correo` | Adaptador | Correo saliente por SMTP (lettre) |
| `crates/infra-seguridad` | Adaptador | Contraseñas Argon2id y segundo factor TOTP |
| `crates/infra-archivos` | Adaptador | Almacén de archivos en disco y enlaces de descarga firmados (S3 previsto, ADR 0003) |
| `crates/infra-planillas` | Adaptador | Lectura de planillas CSV/XLSX para importaciones masivas (ADR 0006) |
| `crates/infra-push` | Adaptador | Notificaciones push de las apps Flutter con Firebase Cloud Messaging (ADR 0011) |
| `crates/shared` | Transversal | Configuración, telemetría, apagado |
| `apps/api` | Entrada | `dad_copropiedad_api`: REST y WebSocket |
| `apps/gateway` | Entrada | `dad_copropiedad_gateway`: webhook y chat de WhatsApp, outbox (correo, WhatsApp y push), workers de recordatorios, gestión, asambleas y control de acceso |
| `apps/puente-acceso` | Entrada | `dad_puente_acceso`: puente de acceso en sitio (cache local, QR sin internet, cola de eventos) |
| `migrations/` | — | Esquema PostgreSQL versionado |

## Estándares y documentación
- [`AGENTS.md`](AGENTS.md): **estándares obligatorios** (arquitectura, reutilización, pruebas, documentación, invariantes). Lo leen todas las herramientas: Claude Code (`CLAUDE.md`), Cursor (`.cursor/rules`), VS Code Copilot (`.github/copilot-instructions.md`), opencode y Codex.
- `make doc`: documentación del código con rustdoc (`target/doc`).
- Contrato de la API REST: [`apps/api/openapi.json`](apps/api/openapi.json), generado con utoipa desde las rutas. Si cambias rutas o DTOs, `make openapi` lo regenera (una prueba falla si queda desactualizado).
- [`docs/adr/`](docs/adr/): decisiones de arquitectura.
