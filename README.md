# dad_copropiedad_back

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
docker compose up -d --wait   # PostgreSQL 18 (+ rol dad_app) y Redis 8
cargo api                     # aplica migraciones y levanta la API en :8080
cargo gateway                 # webhook de WhatsApp en :8081

curl localhost:8080/health    # {"status":"ok",...}
curl localhost:8080/ready     # {"database":"ok","status":"ok"}
```
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
| `crates/shared` | Transversal | Configuración, telemetría, apagado |
| `apps/api` | Entrada | `dad_copropiedad_api`: REST y WebSocket |
| `apps/gateway` | Entrada | `dad_copropiedad_gateway`: webhook, outbox, worker |
| `migrations/` | — | Esquema PostgreSQL versionado |

Las convenciones, las reglas de dependencia y los invariantes de votaciones están en [`AGENTS.md`](AGENTS.md).
