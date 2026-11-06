# AGENTS.md — dad_copropiedad_back

Guía para agentes de código (opencode, Cursor, Claude Code, etc.) y para desarrolladores.
Backend de **DAD_COPROPIEDAD**, software de gestión de propiedad horizontal (Colombia, Ley 675 de 2001).
Primer módulo: **votaciones** en asambleas. **Multi-PH desde el inicio** (cada copropiedad es un tenant).

## Arquitectura
**Monolito modular con arquitectura hexagonal** (puertos y adaptadores): un workspace de Cargo que
compila dos binarios con una sola base PostgreSQL. Nada de microservicios a esta escala.

```
crates/
  domain/          entidades, máquinas de estado, quórum, coeficientes. Rust puro, SIN IO.
  application/     casos de uso + puertos (traits): RepositorioVotos, Mensajeria, Reloj, Sesiones…
  infra-postgres/  SQLx: pool, migraciones, transacción por tenant (RLS), repositorios
  infra-whatsapp/  Cloud API: firma de Meta, webhook entrante, (próximo) cliente de envío
  infra-pdf/       actas con Typst (pendiente)
  shared/          configuración (env), telemetría, apagado. Sin Axum ni SQL.
apps/
  api/             binario `dad_copropiedad_api`: REST para el front + WebSocket de resultados
  gateway/         binario `dad_copropiedad_gateway`: webhook WhatsApp, enrutamiento, outbox, limitador 80 mps
migrations/        migraciones SQLx reversibles (*.up.sql / *.down.sql)
docker/            scripts de inicialización de PostgreSQL para desarrollo
```

Reglas de dependencia (**no romper**):
```
domain  ←  application  ←  infra-*  ←  apps/*
```
- `domain`: no depende de ningún crate interno ni de Tokio/SQLx/Axum/HTTP.
- `application`: solo de `domain`. Define los puertos; no conoce SQL, HTTP ni WhatsApp.
- `infra-*`: implementan puertos de `application`. Nunca dependen de `apps/*` ni entre sí.
- `apps/*`: raíz de composición (conectan adaptadores con casos de uso). Handlers sin reglas de negocio.
- `gateway` hace de worker al comienzo (envíos y actas); se separa solo si la carga lo pide.
- Nuevo contexto (cartera, PQRS…) = módulos nuevos en `domain`/`application` (o crates propios si crecen) + router montado en `api_v1()`.

## Stack
- Rust 1.99 (`rust-toolchain.toml`), edición 2024. Axum 0.8 + Tokio · SQLx 0.9 (rustls) · Serde · Tracing · thiserror/anyhow.
- PostgreSQL 18 y Redis 8 vía `docker-compose.yml`. Redis **solo** para sesiones de conversación y pub/sub de resultados en vivo; la verdad vive en PostgreSQL.
- Previstos (agregar cuando se usen): `governor` (80 mps por línea), `utoipa` (OpenAPI para el front), cliente Redis, cliente HTTP para Cloud API, Typst.

## Comandos
| Acción | Comando |
|---|---|
| Infraestructura | `docker compose up -d --wait` / `docker compose down` (`-v` borra datos) |
| Ejecutar API | `cargo api` (lee `.env`; migra si `RUN_MIGRATIONS=true`) |
| Ejecutar gateway | `cargo gateway` |
| Recarga continua | `bacon` (check/clippy) · `bacon run` |
| Pruebas | `cargo pruebas` (nextest; las de BD requieren PostgreSQL arriba) |
| Lint estricto | `cargo lint` (clippy `-D warnings`) |
| Formato | `cargo fmt --all` |
| Cobertura | `cargo cov` → `target/llvm-cov/html/index.html` |
| Migraciones | `sqlx migrate add -r <nombre>` · `sqlx migrate run` · `sqlx migrate revert` |
| Seguridad deps | `cargo deny check` · `cargo audit` |

**Antes de dar una tarea por terminada:** `cargo fmt --all && cargo lint && cargo pruebas` deben pasar sin errores.

## Convenciones de código
- Nombres de dominio en **español** (entidades, funciones de negocio, tablas, columnas); términos técnicos estándar en inglés cuando sea lo natural (`router`, `state`, `config`, `tenant_id`).
- Errores: `DominioError` (domain) → `AppError` (application) → `ApiError` (`apps/api/src/error.rs`, único mapeo a HTTP). `anyhow` solo en `main`/arranque.
- Puertos asíncronos: `fn x(&self, …) -> impl Future<Output = AppResult<T>> + Send` (sin `async-trait`); en la implementación basta `async fn`.
- Casos de uso genéricos sobre sus puertos; se prueban con dobles en memoria (ver `casos_uso/emitir_voto.rs`).
- **Estados como tipos**: transiciones solo en `EstadoReunion::transicionar` / `EstadoPregunta::transicionar`; votar exige una `PreguntaAbierta` (solo la construye `Pregunta::para_votar`).
- IDs tipados (`TenantId`, `PreguntaId`, …): UUID v7, **excepto `VotoId` (UUID v4)**.
- Enums persistidos con `enum_texto!` (`as_str` / `TryFrom<&str>`); el texto es contrato con la BD.
- Prohibido `unwrap()` fuera de pruebas (clippy lo niega). `expect()` solo con mensaje que explique la invariante.
- Sin `unsafe` (prohibido a nivel de workspace).
- Coeficientes y montos: `rust_decimal::Decimal` / `NUMERIC`. **Nunca `f32/f64`.**
- Fechas: `chrono::DateTime<Utc>` / `TIMESTAMPTZ`. Tiempo siempre vía el puerto `Reloj`.
- Logs con `tracing` y campos estructurados; incluir `tenant_id` y `reunion_id` cuando existan. Nunca loguear secretos, URLs de BD, teléfonos, contenido de mensajes ni la opción de un voto secreto.
- Dependencias nuevas: en `[workspace.dependencies]` del `Cargo.toml` raíz y `x.workspace = true` en el crate.

## Base de datos y multi-tenant
- **Tenant = copropiedad.** Toda tabla de negocio lleva `tenant_id` y política RLS `aislamiento_tenant`.
- Dos roles: el **dueño del esquema** (`DATABASE_URL`: migraciones, `sqlx-cli`, `#[sqlx::test]`) y **`dad_app`** (`APP_DATABASE_URL`: lo usan `api` y `gateway`; sin `BYPASSRLS`, no es dueño). Los binarios se niegan a arrancar con un rol privilegiado.
- Todo acceso de negocio dentro de `infra_postgres::transaccion_tenant` (fija `app.tenant_id` con alcance de transacción).
- FK compuestas `(tenant_id, id)`: una fila no puede apuntar a otro tenant.
- Consultas que cruzan tenants (enrutamiento del gateway, consumo del outbox) **solo** mediante funciones `SECURITY DEFINER` que devuelven lo mínimo.
- `lineas_whatsapp` es tabla de plataforma (sin tenant). Una línea o una por asamblea = configuración.
- Toda modificación de esquema = nueva migración reversible. **Nunca editar una migración ya aplicada en otro entorno.**
- Enumeraciones como `TEXT` + `CHECK`. Preferir `sqlx::query!`/`query_as!`; tras cambiar consultas: `cargo sqlx prepare --workspace` y versionar `.sqlx/`. SQL dinámico solo con `AssertSqlSafe` y justificado.

## Invariantes críticos de votaciones (no romper)
1. **Unicidad:** un inmueble vota una vez por pregunta → `UNIQUE (pregunta_id, inmueble_id)` en `participaciones`.
2. **Idempotencia:** `UNIQUE (pregunta_id, idempotency_key)` y `UNIQUE (wamid)` (Meta reintenta webhooks); inserción con `ON CONFLICT DO NOTHING`; un reintento devuelve el resultado original.
3. **Atomicidad:** verificar estado + insertar participación + voto + evento outbox + auditoría en **una sola transacción**. El trigger `validar_participacion` vuelve a exigir `abierta` con `FOR SHARE`.
4. **Secreto del voto** (`tipo_voto = 'secreto'`): `votos` sin `inmueble_id`, sin `participacion_id`, sin marca de tiempo, id UUID v4 (CHECK + trigger). Ni la auditoría ni el outbox (confirmaciones) guardan la opción elegida. No exponer endpoints que crucen participaciones con votos.
5. **Peso por coeficiente:** resultados y quórum suman coeficientes copiados en la fila al votar, no cuentan votos.
6. **Append-only:** `participaciones`, `votos` y `auditoria` rechazan UPDATE/DELETE/TRUNCATE (triggers + permisos).
7. Solo preguntas `abierta` de reuniones `instalada` reciben votos.
8. **Acta:** cada participación guarda hora, canal (`web`/`whatsapp`/`presencial`) y apoderado.

Riesgo residual conocido: el orden físico de inserción en `votos` podría correlacionarse con `participaciones.registrado_en`; evaluar mitigación (lotes/mezcla) antes de producción.

## Mensajería (gateway)
- Webhook: verificar `X-Hub-Signature-256` sobre el cuerpo **crudo** antes de deserializar; responder rápido.
- Sesión por **(línea, teléfono)** (`wa:sesion:{linea}:{telefono}`), resuelta contra las reuniones activas en que el teléfono está inscrito; si hay varias, se pregunta con una lista (`casos_uso::resolver_reunion`).
- Votos con botones (≤ 3 opciones) o listas (≤ 10) — `ContenidoSaliente::elecciones`.
- **Outbox** en lugar de colas externas: el gateway consume con `outbox_reclamar` (`FOR UPDATE SKIP LOCKED`) y marca con `outbox_marcar_enviado` / `outbox_marcar_fallido`. Prioridad: confirmaciones de voto → preguntas nuevas → recordatorios, dentro de 80 mps por línea.
- Resultados en vivo: proyección en memoria (`ProyeccionResultados`) emitida cada ~500 ms por reunión; nunca un mensaje por voto.
- Registro: enlace prellenado con botones, prerregistro y QR en puerta (`inscripciones.metodo`); los tokens se guardan como hash.

## Pruebas
- Reglas de dominio: pruebas unitarias en el mismo archivo (`#[cfg(test)]`).
- Casos de uso: dobles en memoria de los puertos (sin BD).
- Rutas HTTP: `tower::ServiceExt::oneshot` sobre el `router` de cada app.
- Esquema/BD: `#[sqlx::test(migrator = …)]` en `crates/infra-postgres/tests/` (BD aislada por prueba). Operar como la app con `SET LOCAL ROLE dad_app`.
- Concurrencia de votos: pruebas que lancen emisiones simultáneas del mismo inmueble y verifiquen un único voto.

## No hacer
- No subir `.env` ni credenciales. No desactivar lints para "hacer pasar" el build.
- No agregar lógica de negocio en handlers de Axum ni en adaptadores: va en `domain`/`application`.
- No conectar `api`/`gateway` con el dueño del esquema ni con un rol con `BYPASSRLS`.
- No usar `SELECT *` en consultas de la aplicación.
