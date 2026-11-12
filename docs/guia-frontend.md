# Guía de integración para el frontend

Qué necesita saber la web (Vue) y las apps (Flutter) para consumir la API. El **contrato** es
`apps/api/openapi.json` (OpenAPI 3.1): describe cada ruta, parámetro, cuerpo, respuesta, error y
esquema. Esta guía explica lo que el contrato no cuenta por sí solo: los flujos y las
convenciones.

## 1. Obtener el contrato y generar el cliente

| Dónde | Para qué |
|---|---|
| `apps/api/openapi.json` (en este repositorio) | Fuente versionada; cambia en el mismo PR que el backend. |
| `GET /api-docs/openapi.json` | El mismo documento servido por la API (solo con `API_DOCS_HABILITADA=true`). |
| `GET /docs` | Interfaz interactiva (Scalar) para explorar y probar (misma condición). |

Cliente TypeScript tipado:

```bash
npx openapi-typescript ruta/a/openapi.json -o src/api/esquema.d.ts
# y, por ejemplo, openapi-fetch para las llamadas tipadas
```

Los enums tienen esquema con nombre (`EstadoTicket`, `TipoBienPrivado`, …): el cliente genera
uniones (`'abierto' | 'en_revision' | …`), no `string`. Cuando el backend agrega un valor, el
contrato cambia y el compilador del frontend lo señala.

## 2. Convenciones de datos

| Tipo | Formato JSON |
|---|---|
| Identificadores | UUID en texto (`"0193…"`). |
| Fechas y horas | ISO 8601 en **UTC** (`2026-11-12T15:04:05Z`). Mostrar en la zona de la copropiedad (`zona_horaria`, IANA, p. ej. `America/Bogota`), no en la del navegador. |
| Fechas sin hora | `AAAA-MM-DD`. Horas del día: `HH:MM:SS` (hora local de la copropiedad). |
| Montos y coeficientes | **Texto decimal** (`"0.00523000"`, `"150000.00"`). No convertir a `number` para calcular: se pierde precisión. Mostrar con formato, operar con una librería decimal. |
| Opcionales | `null` o ausentes. |

## 3. Rutas

- `/api/v1/auth/…`: registro, sesión, recuperación de contraseña (sin sesión).
- `/api/v1/yo/…`: la cuenta autenticada (sus copropiedades y permisos, sesiones, dispositivos push).
- `/api/v1/copropiedades/{copropiedad_id}/…`: todo lo de una copropiedad. Responde **404** si la
  cuenta no tiene roles en ella (no se revela si existe).
- `/api/v1/organizaciones/{organizacion_id}/…`, `/api/v1/anunciantes/…`: portales de empresas y
  anunciantes.
- `/api/v1/plataforma/…`: operación de la plataforma; exige **segundo factor** activo.

Las copropiedades de la cuenta: `GET /api/v1/yo/copropiedades`. Al entrar a una, pida
`GET /api/v1/copropiedades/{id}/capacidades`: devuelve sus **permisos** efectivos allí
(`Permiso`) y los **módulos** contratados (`Modulo`). Úselos para mostrar u ocultar menús y
botones; no deduzca permisos de los nombres de los roles, porque cada copropiedad los configura.
El backend vuelve a exigirlos: 403 `sin_permiso` o `modulo_no_habilitado`. Vuelva a pedir las
capacidades si recibe uno de esos 403 (pudieron cambiar los roles o el contrato).

## 4. Sesión

1. `POST /api/v1/auth/sesion` con usuario o correo y contraseña. La respuesta (`InicioRespuesta`)
   trae `resultado`:
   - `sesion`: ya vienen los `tokens`.
   - `segundo_factor`: viene un `desafio`; pedir el código TOTP y enviarlos a
     `POST /api/v1/auth/sesion/segundo-factor` (vence en `desafio_expira_en`).
2. `TokensRespuesta`: `token_acceso` (30 min) va en `Authorization: Bearer …`;
   `token_renovacion` se cambia por tokens nuevos en `POST /api/v1/auth/sesion/renovar`.
   - **Cada renovación invalida el token anterior.** Si un token de renovación ya usado vuelve a
     llegar (p. ej. dos pestañas renovando a la vez, o un token robado), se revoca la sesión
     entera y hay que iniciar sesión otra vez. Renueve desde un solo lugar (un *mutex* o una
     sola promesa compartida) y guarde siempre el último token.
   - `expira_maximo_en`: tope absoluto (30 días; 12 h en equipo compartido).
3. Un 401 (`no_autenticado`) con el token de acceso vencido → renovar una vez y repetir la
   petición; si la renovación también da 401 → pantalla de inicio de sesión.
4. Cerrar sesión: `DELETE /api/v1/auth/sesion`; todas las sesiones: `DELETE /api/v1/yo/sesiones`.

Los tokens se muestran una sola vez (el servidor guarda solo su hash). En la web, guárdelos en
memoria y el de renovación en almacenamiento seguro; en Flutter, en el almacén seguro del sistema.

## 5. Errores

Toda respuesta de error tiene la forma `ErrorRespuesta`:

```json
{ "error": { "codigo": "sin_permiso", "mensaje": "texto para mostrar" } }
```

- Decida por `codigo` (catálogo `CodigoError`, con la tabla código → HTTP → significado en el
  contrato), nunca por el texto de `mensaje`.
- `mensaje` es seguro para mostrar al usuario; no trae detalles internos.
- Cada operación documenta sus errores posibles. Comunes: 400 `peticion_invalida` (JSON o
  parámetro mal formado), 401, 403, 404, 409 `conflicto` (ya existe o rompe una regla, como
  quitar el último administrador), 409 `conflicto_estado` (la acción no aplica en el estado actual:
  recargar el recurso) y 422 `validacion`.
- Envíe la cabecera `X-Request-Id` (o lea la que devuelve la API) y muéstrela en los errores
  inesperados: con ella se encuentra el detalle en los logs.

## 6. Reintentos (`Idempotency-Key`)

Los `POST` con sesión aceptan `Idempotency-Key` (1 a 200 caracteres). Genere un UUID por
**acción del usuario** (no por intento) y reutilícelo si la red falla y reintenta: durante 24 h
la API devuelve la respuesta original con `Idempotency-Replayed: true`, sin duplicar nada. La
misma clave con otro cuerpo responde 422 `idempotencia_reusada`. En el voto presencial la
cabecera es obligatoria.

## 7. Listados paginados

`?limite=50&despues=<cursor>` (`limite` de 1 a 200, 50 por defecto). La respuesta
(`PaginaRespuesta_…`) trae `datos` y `siguiente`: pase `siguiente` como `despues` para la página
siguiente; `null` indica que es la última. No hay números de página ni totales (por cursor).

## 8. Límites de peticiones

Las rutas sin sesión (inicio de sesión, recuperación, papeleta, enlaces públicos) tienen límite
por IP. Al superarlo: 429 `demasiadas_peticiones` con `Retry-After` (segundos). Espere ese tiempo
antes de reintentar y muestre un aviso; no reintente en bucle.

## 9. Archivos

- **Subir**: `POST /api/v1/copropiedades/{id}/archivos?nombre=…&clasificacion=…` con el contenido
  crudo en el cuerpo (`Content-Type: application/octet-stream`, no `multipart`). El tipo real se
  valida por contenido; un tipo no admitido o muy grande da 422 `validacion`. La respuesta trae
  el `id` del archivo, que luego se adjunta en otras peticiones (tickets, reservas, actas…).
- **Descargar**: pida `GET …/archivos/{archivo_id}/enlace` y abra la URL que devuelve: es un enlace
  firmado de **5 minutos** que no necesita sesión. Pídalo justo antes de abrirlo; no lo guarde.

## 10. Credenciales sin sesión de persona

| Cabecera | Para qué |
|---|---|
| `X-Papeleta` | Papeleta virtual de una asamblea (el token del enlace personal del asistente). |
| `X-Enlace-Directorio` | Directorio público de la copropiedad (enlace que se comparte en grupos). |
| `X-Pantalla` | Pantalla de publicidad del lobby. |

El token llega en el enlace que se le entrega a la persona; el frontend lo lee de la URL y lo
envía en la cabecera, nunca en la ruta de la API.

## 11. Tablero en vivo de las asambleas (WebSocket)

```js
const ws = new WebSocket(
  `wss://…/api/v1/copropiedades/${id}/asambleas/${reunion}/en-vivo`,
  ["dad.v1", tokenAcceso],
);
ws.onmessage = (e) => pintar(JSON.parse(e.data)); // TableroRespuesta
```

- El token va como **subprotocolo**, nunca en la URL. Exige el permiso `asambleas.ver`.
- El primer mensaje es el tablero completo; luego llega uno nuevo cada vez que cambia algo, a lo
  sumo cada ~500 ms (quórum, punto en curso, escrutinio de las preguntas abiertas). Cada mensaje
  reemplaza al anterior: no hay que acumular diferencias.
- Si el socket se cierra, reconecte con espera creciente y un token de acceso vigente.

## 12. Notificaciones push (apps)

Registre el token de Firebase con `POST /api/v1/yo/dispositivos-push` al iniciar sesión y cuando
Firebase lo renueve; elimínelo con `DELETE /api/v1/yo/dispositivos-push/{dispositivo_id}` al
cerrar sesión. Un token inválido se desactiva solo y el aviso se envía por WhatsApp.

## 13. Cuando cambia el contrato

Cada PR del backend que cambia la API trae el `openapi.json` regenerado. Regenere el cliente,
compile y revise los errores de tipos: un cambio incompatible (campo eliminado o renombrado,
valor de enum retirado) aparece ahí antes de llegar a producción.
