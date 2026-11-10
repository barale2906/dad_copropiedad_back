# ADR 0012 · Cierre para producción: límite de peticiones, auditoría y despliegue

- **Estado:** aceptada
- **Fecha:** 2026-11-09
- **Plan:** `docs/planes/09-cierre-produccion.md`

## Contexto
Antes de operar con PH reales faltaban tres cosas transversales: frenar el abuso de las rutas
sin sesión (fuerza bruta en el inicio de sesión, papeleta, puentes, enlaces de descarga),
consultar la auditoría que todos los módulos ya registran (revisor fiscal, votos reservados) y
un despliegue reproducible en el VPS. La protección de datos se completó en el ADR 0004.

## Decisión
- **Limitador propio, sin crate nuevo** (`apps/api/src/limite.rs`): ventana fija de un minuto
  por (grupo, IP) en memoria. Grupos `auth` (10/min por defecto) y `publico` (120/min). Se
  aplica con extractores en los handlers (`LimiteAuth`, `LimitePublico`), así cada ruta declara
  su límite y las pruebas lo verifican sin IP real. Responde 429 `demasiadas_peticiones` con
  `Retry-After`. Se descartó `governor`: para una instancia basta una tabla con conteo, y con
  varias instancias el límite correcto es el de Nginx (`limit_req`), no uno por proceso.
- **IP del cliente**: de `X-Real-IP` solo con `CONFIAR_PROXY=true` (Nginx la reemplaza); si no,
  la del socket (`into_make_service_with_connect_info`). Un cliente directo no puede falsearla.
- **`AppState::new` crea el limitador sin límites**; `main` lo configura con `LIMITE_*`. Las
  pruebas funcionales siguen deterministas y la del limitador fija sus propios límites.
- **Auditoría de solo lectura** (`GET /copropiedades/{id}/auditoria`, `auditoria.ver`): filtros
  por entidad, id, acción (exacta o prefijo de módulo), actor y fechas. El cursor es el id
  numérico del evento entregado como UUID opaco, para reutilizar `PaginaRespuesta`.
- **Despliegue con contenedores**: `Dockerfile` multi-stage (compila `api`, `gateway` y
  `dad_admin` con la versión fijada de Rust; imagen final `debian:bookworm-slim`, usuario no
  root, `tini`). El CI construye la imagen en cada PR y la publica en GHCR en `main` con
  etiqueta por commit. Las migraciones son un paso explícito (`dad_admin migrar`, servicio
  `migracion` del compose de producción), nunca al arrancar.

## Consecuencias
- Con varias instancias de la API hay que agregar `limit_req` en Nginx.
- El despliegue al VPS sigue siendo manual (`docs/despliegue.md`); automatizarlo exige
  credenciales del VPS en el CI.
- `CONFIAR_PROXY=true` solo detrás de Nginx propio: expuesta directamente, la API confiaría en
  una cabecera del cliente.

## Adenda (plan 10, fase 6): `Idempotency-Key`

- Middleware de la API (`api::idempotencia`) para los `POST` con sesión y la cabecera
  `Idempotency-Key`: la primera petición reserva la clave por cuenta (tabla de plataforma
  `peticiones_idempotentes`, huella SHA-256 de método, ruta y cuerpo) y guarda su respuesta
  (≤ 1 MB, nunca las 5xx). Un reintento recibe la respuesta original con
  `Idempotency-Replayed: true`; otra petición con la misma clave, 422 `idempotencia_reusada`; si la
  primera sigue en curso, 409. Las claves valen 24 h; el worker de gestión del gateway las purga.
- Los votos (`…/votos`) quedan fuera del middleware: su clave es parte del voto (AGENTS.md §9,
  invariante 2) y el reintento responde `registrado: false` desde la transacción.
- Es general y no reemplaza las idempotencias de negocio ya existentes (`idempotency_key` de los
  tickets, `wamid` de WhatsApp, clave del voto), que siguen siendo la garantía dentro de la BD.

