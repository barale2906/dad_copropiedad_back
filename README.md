# dad_copropiedad_back

[![CI](https://github.com/barale2906/dad_copropiedad_back/actions/workflows/ci.yml/badge.svg)](https://github.com/barale2906/dad_copropiedad_back/actions/workflows/ci.yml)

Backend de **DAD_COPROPIEDAD**, el software de gestión de propiedad horizontal de DAD AI & SYSTEMS.
El primer módulo es **votaciones** en asambleas: multi-PH, con votos por web y por WhatsApp (botones o listas).

El backend es un monolito modular con arquitectura hexagonal, hecho con Rust, Axum, SQLx, PostgreSQL y Redis.

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
docker compose up -d --wait   # PostgreSQL 18 (+ rol dad_app), Redis 8 y Mailpit
cargo api                     # aplica migraciones y levanta la API en :8080
cargo gateway                 # webhook de WhatsApp en :8081

curl localhost:8080/health    # {"status":"ok",...}
curl localhost:8080/ready     # {"database":"ok","status":"ok"}
```
Correos en desarrollo: `docker compose up -d` también levanta **Mailpit**; todo lo que envíe el
gateway aparece en <http://localhost:8025> (no sale a internet).

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
| `crates/shared` | Transversal | Configuración, telemetría, apagado |
| `apps/api` | Entrada | `dad_copropiedad_api`: REST y WebSocket |
| `apps/gateway` | Entrada | `dad_copropiedad_gateway`: webhook, outbox, worker |
| `migrations/` | — | Esquema PostgreSQL versionado |

## Estándares y documentación
- [`AGENTS.md`](AGENTS.md): **estándares obligatorios** (arquitectura, reutilización, pruebas, documentación, invariantes). Lo leen todas las herramientas: Claude Code (`CLAUDE.md`), Cursor (`.cursor/rules`), VS Code Copilot (`.github/copilot-instructions.md`), opencode y Codex.
- `make doc`: documentación del código con rustdoc (`target/doc`).
- Contrato de la API REST: [`apps/api/openapi.json`](apps/api/openapi.json), generado con utoipa desde las rutas. Si cambias rutas o DTOs, `make openapi` lo regenera (una prueba falla si queda desactualizado).
- [`docs/adr/`](docs/adr/): decisiones de arquitectura.
