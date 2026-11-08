# AGENTS.md — dad_copropiedad_back

**Fuente única de verdad** de estándares para todo el código de este repositorio, sin importar
quién o qué lo genere: personas, Claude Code, Cursor, VS Code (Copilot), opencode, Codex o la
terminal. Los demás archivos de reglas (`CLAUDE.md`, `.cursor/rules/*.mdc`,
`.github/copilot-instructions.md`) solo **remiten a este documento**; no duplican reglas.

Backend de **DAD_COPROPIEDAD**, software de gestión de propiedad horizontal (Colombia, Ley 675 de 2001).
Primer módulo: **votaciones** en asambleas. **Multi-PH desde el inicio** (cada copropiedad es un tenant).

> ### Premisa de mantenimiento (obligatoria)
> Este documento se actualiza **en el mismo commit/PR** cada vez que se incorpore al proyecto una
> tecnología, lenguaje, servicio externo, dependencia relevante, herramienta o convención nueva
> (o se retire una). Como mínimo: actualizar [Tecnologías](#tecnologías), los comandos y las
> convenciones afectadas. Un cambio que introduce tecnología sin actualizar este archivo **no está
> terminado**. Si una regla de aquí entra en conflicto con lo que pide una tarea, se señala el
> conflicto en lugar de ignorar la regla.

---

## 1. Principios de calidad
Todo código debe ser **profesional, limpio, reutilizable, escalable y reproducible**:

| Principio | Qué significa aquí |
|---|---|
| Limpio | Nombres expresivos (dominio en español), funciones cortas con una responsabilidad, sin código muerto ni comentarios obsoletos, `cargo fmt` y `clippy` sin advertencias. |
| Reutilizable | Antes de escribir, **buscar lo existente** (§3). Lógica común en el crate de la capa correcta, no copiada entre handlers o adaptadores. |
| Escalable | Respetar las capas hexagonales (§2); dependencias por traits (puertos); nada de estado global mutable; operaciones idempotentes; consultas con índices. |
| Reproducible | Toolchain fijado (`rust-toolchain.toml`), `Cargo.lock` versionado, infraestructura en `docker-compose.yml`, migraciones versionadas, `.sqlx/` versionado, configuración solo por variables de entorno. |
| Seguro | Mínimo privilegio, validación en la frontera, secretos fuera del código, RLS multi-tenant, sin datos personales en logs. |
| Simple | La solución más simple que cumpla los requisitos (YAGNI). No introducir abstracciones, crates ni infraestructura "por si acaso". |

Principios de diseño: SOLID aplicado a Rust (traits pequeños y enfocados, inversión de dependencias
mediante puertos), DRY sin abstraer prematuramente (la regla de tres), *parse, don't validate*
(tipos que solo pueden construirse válidos: `Coeficiente`, `ClaveIdempotencia`, `PreguntaAbierta`),
hacer **imposibles los estados inválidos** con el sistema de tipos, errores explícitos con `Result`.

## 2. Arquitectura
**Monolito modular con arquitectura hexagonal** (puertos y adaptadores): un workspace de Cargo que
compila dos binarios con una sola base PostgreSQL. Nada de microservicios a esta escala.

```
crates/
  domain/          entidades, máquinas de estado, quórum, coeficientes. Rust puro, SIN IO.
  application/     casos de uso + puertos (traits): RepositorioVotos, Mensajeria, Reloj, Sesiones…
  infra-postgres/  SQLx: pool, migraciones, transacción por tenant (RLS), repositorios
  infra-whatsapp/  Cloud API: firma de Meta, webhook entrante, cliente de envío y descarga de fotos
  infra-pdf/       actas con Typst (pendiente)
  infra-correo/    correo saliente por SMTP (lettre): puerto EnvioCorreo
  infra-archivos/  almacén de archivos (disco local; S3 previsto) y enlaces firmados: puerto AlmacenArchivos
  infra-seguridad/ contraseñas Argon2id y segundo factor TOTP: puertos HashContrasenas y SegundoFactor
  infra-planillas/ lectura de CSV/XLSX para importaciones masivas: puerto LectorPlanillas
  shared/          configuración (env), telemetría, apagado. Sin Axum ni SQL.
apps/
  api/             binario `dad_copropiedad_api`: REST para el front + WebSocket de resultados
  gateway/         binario `dad_copropiedad_gateway`: webhook WhatsApp, enrutamiento, outbox, limitador 80 mps
migrations/        migraciones SQLx reversibles (*.up.sql / *.down.sql)
docker/            scripts de inicialización de PostgreSQL para desarrollo
```

Reglas de dependencia (**no romper**; Cargo las hace cumplir):
```
domain  ←  application  ←  infra-*  ←  apps/*
```
- `domain`: no depende de ningún crate interno ni de Tokio/SQLx/Axum/HTTP.
- `application`: solo de `domain`. Define los puertos; no conoce SQL, HTTP ni WhatsApp.
- `infra-*`: implementan puertos de `application`. Nunca dependen de `apps/*` ni entre sí.
- `apps/*`: raíz de composición (conectan adaptadores con casos de uso). Handlers sin reglas de negocio.
- ¿Dónde va el código? Regla de negocio → `domain`. Orquestación / "qué pasa cuando…" → `application`.
  Tecnología concreta → `infra-*`. Traducción HTTP/webhook ↔ caso de uso → `apps/*`.
- `gateway` hace de worker al comienzo (envíos y actas); se separa solo si la carga lo pide.
- Nuevo contexto (cartera, PQRS…) = módulos nuevos en `domain`/`application` (o crates propios si crecen) + router montado en `api_v1()`.

## 3. Flujo obligatorio para nueva funcionalidad
1. **Entender** el requisito y sus invariantes (§9). Si algo es ambiguo o afecta reglas legales/negocio, preguntar.
2. **Buscar qué reutilizar antes de escribir código nuevo.** Revisar como mínimo:
   - tipos e IDs en `crates/domain/src/` (¿ya existe la entidad, el enum, el value object?);
   - puertos en `crates/application/src/puertos/` y casos de uso en `casos_uso/`;
   - utilidades en `crates/shared/` y adaptadores en `crates/infra-*`;
   - macros internas (`enum_texto!`, `id_entidad!`), errores existentes, migraciones y funciones SQL;
   - dependencias ya declaradas en `[workspace.dependencies]`.
   Búsqueda sugerida: `grep -rn "<concepto>" crates apps migrations`. Si se reutiliza o se extiende algo, decirlo en el resumen; si se decide **no** reutilizar, justificarlo.
3. **Diseñar por capas**: tipos y reglas en `domain` → puerto(s) y caso de uso en `application` → adaptador en `infra-*` → endpoint en `apps/*`.
4. **Escribir las pruebas** junto con el código (§6), idealmente primero las del dominio.
5. **Documentar** (§7): rustdoc de lo público, anotaciones OpenAPI del endpoint, `AGENTS.md`/`README.md` si cambian comandos, arquitectura o tecnologías.
6. **Verificar**: `make verificar` (fmt + clippy + pruebas) y, si hay dependencias nuevas, `make deny`.
7. **Cumplir la [definición de terminado](#11-definición-de-terminado).**

## 4. Tecnologías
Al agregar o retirar una tecnología, actualizar esta tabla (ver premisa).

| Área | Tecnología | Estado | Notas |
|---|---|---|---|
| Lenguaje backend | Rust 1.99, edición 2024 | En uso | Fijado en `rust-toolchain.toml`. |
| HTTP / WebSocket | Axum 0.8 + Tokio + tower-http | En uso | Middlewares: request-id, trace, timeout, límite de cuerpo. |
| Persistencia | PostgreSQL 18 + SQLx 0.9 (rustls) | En uso | Consultas verificadas en compilación; RLS por tenant. Extensiones `unaccent` y `pg_trgm` (búsqueda normalizada) y `btree_gist` (reservas y asignaciones sin solapes con `EXCLUDE`). |
| Caché / tiempo real | Redis 8 | Infra lista | Solo pub/sub de resultados de asambleas (plan 06). Las sesiones del chat viven en PostgreSQL (`sesiones_chat`, ADR 0009). |
| Serialización / errores / logs | Serde · thiserror/anyhow · tracing | En uso | |
| Decimales / fechas / IDs | rust_decimal · chrono · chrono-tz · uuid (v7, v4 para votos) | En uso | `chrono-tz`: los horarios de cada PH se evalúan en su zona IANA (`copropiedades.zona_horaria`). |
| Criptografía | hmac + sha2 + sha1 + hex · argon2 · getrandom | En uso | Firma de webhooks de Meta, enlaces firmados, SHA-256 de archivos y tokens; Argon2id para contraseñas; TOTP (HMAC-SHA1, RFC 6238) en `infra-seguridad`. |
| Autenticación | Tokens opacos propios + TOTP | En uso | Usuario o correo + contraseña; acceso 30 min, renovación rotativa con detección de reutilización (ADR 0005). Sin JWT. |
| Mensajería | WhatsApp Cloud API (Meta) | En uso | Detrás del puerto `Mensajeria`; BSP intercambiable. Envío con plantillas aprobadas fuera de la ventana de 24 h (`ticket_actualizado`, `reporte_urgente`). Variables `WHATSAPP_*`. |
| Cliente HTTP saliente | reqwest 0.13 · rustls (ring) · webpki-roots | En uso | Solo en `infra-whatsapp` (Graph API). TLS sin OpenSSL ni aws-lc; el proveedor de criptografía se fija al construir el cliente. |
| Correo | lettre 0.11 (SMTP, rustls) · SMTP de HestiaCP en producción · Mailpit en desarrollo | En uso | Crate `infra-correo`, puerto `EnvioCorreo`; variables `SMTP_*`. Mailpit atrapa los correos locales (bandeja en `localhost:8025`). |
| Archivos | Disco del VPS (`ARCHIVOS_DIR`) · S3 previsto | En uso | Crate `infra-archivos`, puerto `AlmacenArchivos`; clave lógica + columna `almacen`; descargas con enlace HMAC de 5 min. Metodología de migración en ADR 0003. |
| Planillas | csv 1.4 · calamine 0.38 | En uso | Crate `infra-planillas`, puerto `LectorPlanillas` (solo lectura, Rust puro). Importación con vista previa (ADR 0006). |
| Limitador | Ritmo por línea propio (`apps/gateway/src/mensajeria.rs`) | En uso | 80 mensajes/s por línea de WhatsApp; `governor` solo si hace falta limitar endpoints públicos. |
| OpenAPI | utoipa 6 + utoipa-axum + utoipa-scalar | En uso | Rutas registradas con `routes!` (contrato = rutas servidas). UI en `/docs` solo con `API_DOCS_HABILITADA=true`. Contrato versionado en `apps/api/openapi.json` (§7). |
| Actas | Typst | Previsto | En `infra-pdf`. |
| Contenedores | Docker Compose (dev) | En uso | Imagen de producción construida en CI. |
| CI/CD | GitHub Actions | En uso | `.github/workflows/ci.yml`: fmt, clippy, pruebas con PostgreSQL, rustdoc, cargo-deny. Pendiente: build de imagen y despliegue. |
| Despliegue | VPS Ubuntu + Nginx (HestiaCP) | Previsto | Binarios nativos/contenedor detrás de Nginx con TLS. |
| Calidad | rustfmt · clippy · nextest · llvm-cov · cargo-deny · cargo-audit · bacon | En uso | |
| Frontend (otro repo) | Vue 3 + TypeScript (Angular posible) | Externo | Consume el OpenAPI y genera su cliente tipado. |
| Apps móviles (otro repo) | Android + iOS | Previsto | Mismo contrato OpenAPI que la web; notificaciones push. |
| IA / analítica (futuro) | Python | Externo | Servicios aparte; integración por API/eventos, nunca tablas compartidas. |

## 5. Convenciones de código
- Nombres de dominio en **español** (entidades, funciones de negocio, tablas, columnas, pruebas); términos técnicos estándar en inglés cuando sea lo natural (`router`, `state`, `config`, `tenant_id`).
- **Glosario de la Ley 675 de 2001** (ADR 0002): en código, BD, API, documentación e interfaz se usan los términos de la ley. Copropiedad / propiedad horizontal = el tenant (`copropiedad`, `TenantId`); **bien privado** = unidad con coeficiente (`bienes_privados`, `BienPrivadoId`; nunca "inmueble" ni "unidad"); **bien común** (esencial, no esencial, de uso exclusivo); **expensas comunes**; **coeficiente de copropiedad**; **propietario**, **tenedor** (arrendatario u otro título) y **residente**; asamblea general de propietarios, consejo de administración, administrador, revisor fiscal, comité de convivencia; reglamento de propiedad horizontal. Lo que la ley no define usa un término técnico neutro documentado (`agrupacion`, `cuenta`, `organizacion`). Las etiquetas cotidianas ("Torre 3", "Apto 506") son valores configurables, no nombres de tipos.
- Errores: `DominioError` (domain) → `AppError` (application) → `ApiError` (`apps/api/src/error.rs`, único mapeo a HTTP). `anyhow` solo en `main`/arranque. Mensajes de error al cliente sin detalles internos.
- Puertos asíncronos: `fn x(&self, …) -> impl Future<Output = AppResult<T>> + Send` (sin `async-trait`); en la implementación basta `async fn`.
- Casos de uso genéricos sobre sus puertos; se prueban con dobles en memoria (ver `casos_uso/emitir_voto.rs`).
- **Unidad de trabajo y auditoría**: un caso de uso que modifica datos abre una transacción con el puerto `UnidadDeTrabajo` (implementación `infra_postgres::UnidadDeTrabajoPg`, que fija el tenant), hace el cambio con los puertos que implementa la transacción, registra un `EventoAuditoria` con `RegistroAuditoria` y llama a `confirmar()`; sin `confirmar` todo se revierte. Acciones como constantes `AccionAuditoria::new("modulo.recurso.verbo")` (un formato inválido no compila). `DetalleAuditoria` rechaza claves de datos personales o del contenido de un voto. Dobles en memoria: `puertos::unidad_trabajo::memoria`.
- **Permisos**: todo caso de uso que lee o modifica datos de negocio llama a `ctx.exigir(Permiso::…)` (`AppError::SinPermiso` → 403 `sin_permiso`). `Contexto::new(tenant)` es el contexto del **sistema** (todos los permisos de copropiedad); las peticiones usan `Contexto::de_cuenta`. Un permiso nuevo = variante en `domain::Permiso` + migración que lo inserte en `permisos` (y en las plantillas que correspondan); la prueba `catalogo_de_permisos_y_plantillas_coinciden_con_el_codigo` lo exige. Nadie otorga lo que no tiene (no escalamiento).
- **Dos ámbitos de transacción**: `UnidadDeTrabajo` (copropiedad, `app.tenant_id`) y `UnidadDeTrabajoPlataforma` (cuentas, sesiones, roles de plataforma, organizaciones; `app.plataforma = on`). Nunca leer cuentas o sesiones desde una transacción de copropiedad: lo que cruce ámbitos va por una función `SECURITY DEFINER`.
- **Estados como tipos**: transiciones solo en `EstadoReunion::transicionar` / `EstadoPregunta::transicionar`; votar exige una `PreguntaAbierta` (solo la construye `Pregunta::para_votar`).
- IDs tipados (`TenantId`, `PreguntaId`, …): UUID v7, **excepto `VotoId` (UUID v4)**.
- Enums persistidos con `enum_texto!` (`as_str` / `TryFrom<&str>`); el texto es contrato con la BD.
- Prohibido `unwrap()` fuera de pruebas (clippy lo niega). `expect()` solo con mensaje que explique la invariante. Sin `unsafe` (prohibido en el workspace). Sin `todo!()` en código entregado.
- Coeficientes y montos: `rust_decimal::Decimal` / `NUMERIC`. **Nunca `f32/f64`.**
- Fechas: `chrono::DateTime<Utc>` / `TIMESTAMPTZ`. Tiempo siempre vía el puerto `Reloj`.
- Visibilidad mínima: `pub` solo lo que otro crate necesita; `pub(crate)` o privado por defecto.
- Logs con `tracing` y campos estructurados; incluir `tenant_id` y `reunion_id` cuando existan. Nunca loguear secretos, URLs de BD, teléfonos, contenido de mensajes ni la opción de un voto secreto.
- Configuración solo por variables de entorno (`crates/shared/src/config.rs`); toda variable nueva va también en `.env.example` con comentario. Los `Debug` de configuración ocultan secretos.
- Dependencias nuevas: justificar la necesidad, preferir crates mantenidos y con licencia permitida en `deny.toml`, declararlas en `[workspace.dependencies]` del `Cargo.toml` raíz (versión exacta) y usar `x.workspace = true`. Pasar `make deny`. Actualizar §4.

## 6. Pruebas
Toda funcionalidad nueva o modificada se entrega **con pruebas automatizadas que pasan**. Se exigen
dos familias complementarias:

- **Pruebas de código (unitarias y de integración):** verifican la *implementación* —que cada
  función, tipo, regla, adaptador y consulta haga exactamente lo que debe, incluidos los casos
  límite y los errores.
- **Pruebas funcionales (de comportamiento / aceptación):** verifican la *funcionalidad desde la
  perspectiva de quien la usa* —un cliente de la API REST, Meta enviando un webhook, un votante—
  ejecutando el flujo completo y comprobando el resultado observable (código HTTP, cuerpo, filas
  persistidas, mensaje encolado), sin depender de detalles internos.

| Nivel | Qué valida | Dónde / cómo |
|---|---|---|
| Unitarias de dominio | Reglas puras: estados, quórum, mayorías, coeficientes, secreto del voto | `#[cfg(test)]` en el mismo archivo de `crates/domain` |
| Casos de uso | Orquestación y errores de aplicación | `crates/application`, con dobles en memoria de los puertos |
| Integración de BD | Esquema, RLS, triggers, restricciones, repositorios | `crates/infra-postgres/tests/` con `#[sqlx::test(migrator = …)]`; operar como la app con `SET LOCAL ROLE dad_app` |
| Adaptadores externos | Firma, parseo y normalización de payloads | `crates/infra-*` con fixtures reales (JSON de ejemplo de Meta) |
| Funcionales de API | Endpoint completo: petición → respuesta y efectos | `tower::ServiceExt::oneshot` sobre el `router` de cada app (`apps/*/src/routes`); con BD real vía `#[sqlx::test]` cuando aplique |
| Concurrencia | Emisiones simultáneas del mismo bien privado → un único voto | `tokio::spawn` + `#[sqlx::test]` |
| Contrato | El OpenAPI generado coincide con `apps/api/openapi.json` | `apps/api/tests/contrato_openapi.rs`; regenerar a propósito con `make openapi` |

Reglas:
- Cada regla de negocio tiene al menos un caso positivo y uno negativo; cada invariante de §9 tiene una prueba que fallaría si se rompe.
- Pruebas deterministas: sin dependencia del reloj real (puerto `Reloj`), del orden ni de la red externa.
- Nombre descriptivo en español que diga el comportamiento: `secreto_no_deja_rastro_del_bien_privado`.
- Patrón Preparar / Ejecutar / Verificar; un comportamiento por prueba; helpers y fixtures reutilizables en lugar de copiar.
- Un bug corregido llega con la prueba que lo reproduce.
- Cobertura orientativa ≥ 80 % en `domain` y `application` (`make cov`); el número no sustituye probar los casos importantes.
- `make test-rapido` corre sin BD; `make test` requiere `make up`.

## 7. Documentación
El código se documenta para poder **generar** la documentación de referencia y la de la API REST.

### Herramientas (equivalentes en Rust)
| Necesidad | Equivalente PHP | Herramienta Rust | Comando / salida |
|---|---|---|---|
| Documentación del código | PHPDoc / phpDocumentor | **rustdoc** (comentarios `///` y `//!`, Markdown) | `make doc` → `target/doc/<crate>/index.html` |
| Ejemplos verificados | — | **doctests** (bloques de código en `///` que se compilan y ejecutan) | `cargo test --doc` |
| Especificación OpenAPI | swagger-php / L5-Swagger | **utoipa** (`#[derive(ToSchema)]`, `#[utoipa::path]`) | `/api-docs/openapi.json` |
| UI interactiva de la API | Swagger UI | **utoipa-swagger-ui**, **utoipa-scalar** o **utoipa-redoc** | `/docs` (solo en entornos no productivos o protegida) |
| Cliente del frontend | — | **openapi-typescript** / **openapi-generator** (en el repo del front) | Cliente TS tipado desde el OpenAPI |
| Guías y manuales | — | **mdBook** (si hace falta un sitio de guías) | `docs/` → sitio estático |

Alternativa a utoipa: `aide` (genera OpenAPI desde los tipos de Axum). Se elige **utoipa** por madurez y adopción; al incorporarlo, actualizar §4.

### Reglas
- Todo ítem `pub` lleva `///`: qué hace, **por qué** existe si no es obvio, invariantes, errores (`# Errors`) y pánicos (`# Panics`) si los hay. Cada crate y módulo lleva `//!` con su propósito y su lugar en la arquitectura.
- Ejemplos en doctests cuando el uso no sea evidente; deben compilar.
- Enlaces internos con `[`Tipo`]` (intra-doc links); `make doc` debe pasar sin advertencias.
- **Cada endpoint REST** se anota con `#[utoipa::path]`: método, ruta, parámetros, cuerpo, respuestas (incluidos errores con su `codigo`), seguridad y etiqueta del módulo. Cada DTO de entrada/salida deriva `ToSchema` con descripciones y ejemplos. Un endpoint sin documentación OpenAPI no está terminado.
- DTOs de la API viven en `apps/api` (no se exponen entidades de dominio directamente); versionado en la ruta (`/api/v1`).
- Rutas: `/api/v1/copropiedades/{copropiedad_id}/…` (recursos de una copropiedad), `/api/v1/yo/…` (cuenta autenticada), `/api/v1/plataforma/…` (global). Cada handler se registra con `utoipa_axum::routes!` en su módulo de `apps/api/src/routes/`.
- Otras rutas: `/api/v1/auth/…` (sin sesión: registro, inicio de sesión), `/api/v1/organizaciones/{organizacion_id}/…`, `/api/v1/permisos` y `/api/v1/archivos/descarga` (sin sesión: el enlace firmado es la credencial, ADR 0003).
- Autorización en los handlers con los extractores de `api::sesion`: `CuentaAutenticada` (`/yo`, `/organizaciones`), `OperadorPlataforma` (`/plataforma`, exige segundo factor) y `EnCopropiedad` (`/copropiedades/{copropiedad_id}`, carga los permisos; 404 si la cuenta no tiene roles allí). Cada endpoint protegido declara `security(("bearer" = []))`.
- Handlers con los extractores de `api::extractores` (`Json`, `Query`, `Path`), nunca los de `axum` directamente: así una petición mal formada también responde con el error uniforme (`peticion_invalida`).
- Listados paginados por cursor: `ParametrosPagina` (`?limite=&despues=`) → `application::SolicitudPagina` → el repositorio consulta `filas_a_consultar()` → `Pagina::desde_filas` → `PaginaRespuesta<T>`. Nunca `OFFSET`.
- Formato de error uniforme: `{ "error": { "codigo": "...", "mensaje": "..." } }` (`ErrorRespuesta`). Los códigos viven en `CodigoError` (`apps/api/src/error.rs`) con su tabla código → HTTP → significado; un código nuevo se agrega ahí y en la tabla.
- Comentarios de línea solo para explicar el *porqué* (decisiones, normas legales, riesgos), no el *qué*.
- Decisiones de arquitectura relevantes: registrar un ADR breve en `docs/adr/NNNN-titulo.md` (contexto, decisión, consecuencias).
- `README.md`: cómo arrancar y operar. `AGENTS.md`: estándares. Mantener ambos al día.

## 8. Base de datos y multi-tenant
- **Tenant = copropiedad.** Toda tabla de negocio lleva `tenant_id` y política RLS `aislamiento_tenant`.
- Dos roles: el **dueño del esquema** (`DATABASE_URL`: migraciones, `sqlx-cli`, `#[sqlx::test]`) y **`dad_app`** (`APP_DATABASE_URL`: lo usan `api` y `gateway`; sin `BYPASSRLS`, no es dueño). Los binarios se niegan a arrancar con un rol privilegiado.
- Todo acceso de negocio dentro de `infra_postgres::transaccion_tenant` (fija `app.tenant_id` con alcance de transacción).
- FK compuestas `(tenant_id, id)`: una fila no puede apuntar a otro tenant.
- Consultas que cruzan tenants (enrutamiento del gateway, consumo del outbox) **solo** mediante funciones `SECURITY DEFINER` (`SET search_path`, `REVOKE … FROM PUBLIC`) que devuelven lo mínimo.
- `lineas_whatsapp` es tabla de plataforma (sin tenant). Una línea o una por asamblea = configuración.
- Toda modificación de esquema = nueva migración reversible (`make migracion NOMBRE=…`) probada con up → down → up. **Nunca editar una migración ya aplicada en otro entorno.**
- **Archivos** (ADR 0003): nadie toca rutas de disco; todo pasa por `AlmacenArchivos` con una `ClaveArchivo`. La fila de `archivos` guarda la clave lógica y el `almacen`, nunca rutas absolutas. El tipo se valida por contenido (`TipoArchivo::validar`), nunca por extensión. El nombre original no va a logs ni a la auditoría. Casos de uso en `casos_uso::archivos` (`subir_archivo`, `enlace_descarga`, `migrar_lote`).
- **Datos personales** (ADR 0004): toda columna con datos personales lleva `COMMENT ON COLUMN … IS 'dato_personal: …'` (o `dato_sensible:`) en su migración y se agrega a la prueba `columnas_personales_estan_clasificadas`. Antes de tratar datos para comunicaciones o publicidad se consulta `casos_uso::proteccion_datos::esta_autorizado`.
- **Búsqueda**: por texto con `normalizar(columna)` (sin tildes ni mayúsculas) y un índice GIN `gin_trgm_ops` sobre esa expresión; nunca `ILIKE` sobre la columna cruda. Ordenar listados por nombre con `normalizar(...)`.
- **Coeficientes** (ADR 0006): nunca se editan uno por uno. Se prepara un `cuadros_coeficientes` borrador, se revisa (`RevisionCuadro`) y se activa; la suma es exactamente 1 (sin tolerancia) y `bienes_privados.coeficiente` solo lo escribe la activación (trigger). No se activa con una reunión `instalada`.
- **Titulares**: los vigentes de un bien suman 100 % por clase de derecho y tienen un representante (`ConjuntoTitulares` + trigger diferido). Cambiar de dueño = `transferir_titularidad` (cierra y abre en una transacción); el rol `propietario` se deriva de la titularidad (origen `propiedad`), nunca se asigna a mano.
- **Censo** (ADR 0007): residencias con calidad y un responsable vigente por bien; el rol `residente` se deriva de la residencia (origen `residencia`) y vence al terminarla en la misma transacción. Placas normalizadas (`domain::Placa`) y únicas entre las vigentes. Fecha de nacimiento y movilidad reducida son `dato_sensible`: se ocultan sin `personas.datos_sensibles.ver`. Lo que proponen residentes y propietarios entra a `cambios_censo` y solo cambia el censo al aprobarlo.
- **Reservas** (ADR 0008): todo recurso reservable (bien común, parqueadero, agenda, herramienta) usa el mismo motor (`casos_uso::reservas`) y su `PoliticaReserva`. Horarios en la hora local de la PH. Exclusivos: `EXCLUDE` en la BD; con aforo: bloquear el recurso y contar personas. Solicitar es idempotente. Montos liquidados en `Decimal`; penalidades y daños como `cargos_pendientes`. Los casos de uso del módulo exigen `exigir_modulo_en(.., Modulo::BienesComunes)` dentro de su transacción.
- **Tickets** (ADR 0009): estados solo por `EstadoTicket::transicionar` con evento en `ticket_eventos`; número `PQR-AAAA-NNNNN` por PH y año desde `consecutivos`; plazos con `domain::ticket::vencimiento` (Ley 1755 por defecto, configurables); radicar es idempotente por clave (el `wamid` en WhatsApp). El solicitante nunca ve las notas internas; en `reporte_convivencia` la identidad solo con `tickets.convivencia.ver`. Avisos internos con `casos_uso::avisos` (destinatarios por permiso), nunca a direcciones fijas.
- **Mantenimiento** (ADR 0009): una orden preventiva por (plan, fecha) (`ux_orden_por_periodo`); la baja de un activo exige soporte y es definitiva; quien ejecuta una orden no la verifica. Los historiales (`ticket_mensajes`, `ticket_eventos`, `movimientos_inventario`, `cumplimientos_obligacion`, `conversacion_mensajes`) son append-only.
- **Importaciones** (`casos_uso::importaciones`): archivo → validación fila por fila → vista previa → confirmar → aplicar en una transacción. Una importación nueva reutiliza ese flujo; los errores no repiten el valor de la celda.
- **Módulos contratados**: los casos de uso o routers de un módulo vendible (`domain::Modulo`) exigen `casos_uso::modulos::exigir_modulo`; si no está habilitado → `modulo_no_habilitado` (403).
- Enumeraciones como `TEXT` + `CHECK`; coeficientes `NUMERIC(10,8)`; fechas `TIMESTAMPTZ`; índices para cada consulta frecuente y cada FK usada en filtros.
- Preferir `sqlx::query!`/`query_as!`; tras cambiar consultas: `make sqlx-prepare` y versionar `.sqlx/`. SQL dinámico solo con `AssertSqlSafe`, justificado, y con datos enlazados (`$1`), nunca interpolados desde la entrada del usuario. Prohibido `SELECT *`.

## 9. Invariantes críticos de votaciones (no romper)
1. **Unicidad:** un bien privado vota una vez por pregunta → `UNIQUE (pregunta_id, bien_privado_id)` en `participaciones`.
2. **Idempotencia:** `UNIQUE (pregunta_id, idempotency_key)` y `UNIQUE (wamid)` (Meta reintenta webhooks); inserción con `ON CONFLICT DO NOTHING`; un reintento devuelve el resultado original.
3. **Atomicidad:** verificar estado + insertar participación + voto + evento outbox + auditoría en **una sola transacción**. El trigger `validar_participacion` vuelve a exigir `abierta` con `FOR SHARE`.
4. **Secreto del voto** (`tipo_voto = 'secreto'`): `votos` sin `bien_privado_id`, sin `participacion_id`, sin marca de tiempo, id UUID v4 (CHECK + trigger). Ni la auditoría ni el outbox (confirmaciones) guardan la opción elegida. No exponer endpoints que crucen participaciones con votos.
5. **Peso por coeficiente:** resultados y quórum suman coeficientes copiados en la fila al votar, no cuentan votos.
6. **Append-only:** `participaciones`, `votos` y `auditoria` rechazan UPDATE/DELETE/TRUNCATE (triggers + permisos).
7. Solo preguntas `abierta` de reuniones `instalada` reciben votos.
8. **Acta:** cada participación guarda hora, canal (`web`/`whatsapp`/`presencial`) y apoderado.

Riesgo residual conocido: el orden físico de inserción en `votos` podría correlacionarse con `participaciones.registrado_en`; evaluar mitigación (lotes/mezcla) antes de producción.

### Mensajería (gateway)
- Webhook: verificar `X-Hub-Signature-256` sobre el cuerpo **crudo** antes de deserializar; responder rápido.
- Sesión por **(línea, teléfono)** en PostgreSQL (`sesiones_chat`, ámbito de plataforma). Cualquier línea atiende cualquier PH: la PH se resuelve por el teléfono (`vinculos_por_telefono`); si hay varios vínculos se pregunta con una lista. Cada turno se cierra en una transacción (`SesionesConversacion::cerrar_turno`: sesión + respuestas al outbox + `wamid` procesado); lo que crea datos es idempotente por `wamid`.
- El chat (`casos_uso::chat`) es una máquina de estados en `application`; el webhook responde 200 de inmediato y atiende el turno aparte. Fuera de la ventana de 24 h solo plantillas aprobadas. En asambleas, la sesión se resolverá contra las reuniones activas (`casos_uso::resolver_reunion`, plan 06).
- Votos con botones (≤ 3 opciones) o listas (≤ 10) — `ContenidoSaliente::elecciones`.
- **Outbox multicanal** (`canal`: whatsapp, correo, push) en lugar de colas externas: se encola con `RegistroOutbox` dentro de la transacción del hecho; el worker del gateway (`worker_salida`, caso de uso `despachar_lote`) reclama **solo los canales que sabe enviar** con `outbox_reclamar(canales, límite)` (`FOR UPDATE SKIP LOCKED`) y marca con `outbox_marcar_enviado` / `outbox_marcar_fallido` (reintentos con espera exponencial, `PoliticaReintentos`). Prioridad: confirmaciones → eventos → recordatorios → publicidad (9), dentro de 80 mps por línea en WhatsApp. El texto de error de un envío puede contener el destino: va a la fila del outbox, nunca a los logs.
- Resultados en vivo: proyección en memoria (`ProyeccionResultados`) emitida cada ~500 ms por reunión; nunca un mensaje por voto.
- Registro: enlace prellenado con botones, prerregistro y QR en puerta (`inscripciones.metodo`); los tokens se guardan como hash.

## 10. Seguridad, infraestructura y operación
- **Secretos:** nunca en el código ni en git (`.env` está ignorado); en producción, variables de entorno del servidor/CI. Rotables.
- **Datos personales** (Ley 1581 de 2012): minimizar lo que se guarda, no loguearlo, no enviarlo a servicios externos sin base legal.
- **Entrada:** validar en la frontera y convertir a tipos de dominio; límites de tamaño de cuerpo y timeouts en todos los routers; rate limiting en endpoints públicos.
- **Errores:** el cliente recibe `codigo` + mensaje seguro; el detalle va a logs con `request_id`.
- **Observabilidad:** logs JSON en producción (`LOG_FORMAT=json`), `x-request-id` propagado, sondas `/health` (vivo) y `/ready` (dependencias). Métricas de latencia, errores, reintentos e intentos duplicados cuando se incorporen.
- **Reproducibilidad:** misma versión de Rust en local y CI; `Cargo.lock` versionado; imágenes con versión fija (no `latest`); builds de release desde CI, no desde una máquina personal.
- **Contenedores de producción:** multi-stage, imagen final mínima, usuario no root, sin herramientas de compilación, configuración por entorno.
- **Dependencias:** `make deny` y `make audit` en CI; actualizar de forma deliberada.
- **Apagado ordenado** (SIGTERM) y migraciones de producción ejecutadas como paso explícito de despliegue (no al arrancar).

## 11. Definición de terminado
Una tarea está terminada solo si:
- [ ] Se revisó qué reutilizar (§3) y se respetan las capas y dependencias (§2).
- [ ] `make verificar` pasa (`cargo fmt --check`, `clippy -D warnings`, todas las pruebas).
- [ ] Hay pruebas de código y funcionales para lo nuevo o modificado (§6), incluidos casos de error.
- [ ] Lo público está documentado con rustdoc; los endpoints, con OpenAPI (§7); `make doc` sin advertencias.
- [ ] Cambios de esquema en migración nueva reversible; `.sqlx/` actualizado si cambiaron consultas.
- [ ] Variables nuevas en `.env.example`; dependencias nuevas pasan `make deny`.
- [ ] Ninguna invariante de §9 se debilitó.
- [ ] `AGENTS.md`/`README.md` actualizados si cambió tecnología, comandos, arquitectura o convenciones.
- [ ] El CI (`.github/workflows/ci.yml`) está en verde.

## 12. Comandos
| Acción | Make | Equivalente |
|---|---|---|
| Ver todos los comandos | `make` | — |
| Primer arranque | `make herramientas` · `make setup` | `cp .env.example .env && docker compose up -d --wait && sqlx migrate run` |
| Contenedores | `make up` · `make down` · `make reiniciar-bd` | `docker compose …` |
| Ejecutar | `make api` · `make gateway` · `make dev` | `cargo api` · `cargo gateway` · `bacon` |
| Primer superusuario | `make superusuario USUARIO=admin CORREO=a@b.co` | `DAD_ADMIN_CONTRASENA=… cargo run --bin dad_admin -- crear-superusuario --usuario admin` |
| Verificación completa | `make verificar` | `cargo fmt --all --check && cargo lint && cargo pruebas` |
| Pruebas | `make test` · `make test-rapido` · `make cov` | `cargo pruebas` (nextest) + `cargo test --workspace --doc` (doctests) |
| Documentación | `make doc` | `cargo doc --workspace --no-deps` |
| Contrato OpenAPI | `make openapi` | `ACTUALIZAR_OPENAPI=1 cargo nextest run -p api --test contrato_openapi` |
| Migraciones | `make migracion NOMBRE=x` · `make migrar` · `make revertir` | `sqlx migrate add -r x` … |
| Dependencias | `make deny` · `make audit` | `cargo deny check` · `cargo audit` |

## 13. Git
- Rama principal `main`; trabajo nuevo en ramas `feature/<tema>`, `fix/<tema>`, `docs/<tema>`.
- Commits en español, en imperativo y con un solo propósito (`Agregar repositorio de votos en Postgres`); cuerpo con el *porqué* si no es obvio.
- No subir `.env`, credenciales, `target/` ni archivos generados no versionables.

## 14. No hacer
- No desactivar lints ni pruebas para "hacer pasar" el build.
- No agregar lógica de negocio en handlers de Axum ni en adaptadores: va en `domain`/`application`.
- No conectar `api`/`gateway` con el dueño del esquema ni con un rol con `BYPASSRLS`.
- No duplicar código que ya existe: extender o extraer.
- No incorporar tecnología sin actualizar este documento.
