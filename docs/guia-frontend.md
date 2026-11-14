# Guía de integración para el frontend

Qué necesita saber la web (Vue) y las apps (Flutter) para consumir la API. El **contrato** es
`apps/api/openapi.json` (OpenAPI 3.1): describe cada ruta, parámetro, cuerpo, respuesta, error y
esquema. Esta guía explica lo que el contrato no cuenta por sí solo: los flujos y las
convenciones.

**Cómo usar esta guía**

| Necesito… | Dónde |
|---|---|
| Generar el cliente tipado y conocer los formatos | §1 y §2 |
| Qué ve cada tipo de usuario y en qué entidad está | §3 (entrar a una entidad) y §4 (perfiles) |
| El flujo de un módulo (asambleas, censo, cartera, facturación…) | §3 (entidad, contabilidad, facturación) y §4 (PH) |
| Qué botones mostrar según el estado de un recurso | §5 |
| Sesión, errores, reintentos, paginación, archivos, tiempo real | §6 a §14 |
| Consejos de diseño que aprovechan lo que hace el backend | §15 |
| **Todas las operaciones** con su permiso, cuerpo, respuesta y errores | [guia-frontend-endpoints.md](guia-frontend-endpoints.md) (generado del contrato con `make guia-endpoints`) |
| Probar en vivo | `/docs` en un entorno de desarrollo (§1) |

## 1. Obtener el contrato y generar el cliente

| Dónde | Para qué |
|---|---|
| `apps/api/openapi.json` (en este repositorio) | Fuente versionada; cambia en el mismo PR que el backend. |
| `GET /api-docs/openapi.json` | El mismo documento servido por la API (solo con `API_DOCS_HABILITADA=true`). |
| `GET /docs` | Interfaz interactiva (Scalar) para explorar y probar (misma condición). |

**Dominio de la web.** Si la web se publica en otro dominio que la API (`app.<dominio>` y
`api.<dominio>`), ese origen exacto debe estar en `CORS_ORIGENES` del backend; si no, el navegador
bloquea las llamadas. En desarrollo viene permitido `http://localhost:5173` (Vite). La sesión va en
`Authorization`, nunca en cookies, así que no se usa `credentials: 'include'`. Las cabeceras de
respuesta que puede leer la web son `X-Request-Id`, `Idempotency-Replayed`, `X-Contenido-Sha256`,
`Retry-After` y `Content-Disposition`.

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
- `/api/v1/yo/…`: la cuenta autenticada (sus entidades, perfil y permisos de plataforma, sesiones,
  dispositivos push).
- `/api/v1/entidades/{entidad_id}/…`: lo común de cualquier entidad (capacidades, roles,
  archivos, auditoría, terceros, tributario, contabilidad y facturación). Mismo 404 sin roles.
- `/api/v1/copropiedades/{copropiedad_id}/…`: lo propio de una copropiedad. Responde **404** si la
  cuenta no tiene roles en ella (no se revela si existe).
- `/api/v1/organizaciones/{organizacion_id}/…`, `/api/v1/anunciantes/…`: portales de empresas y
  anunciantes.
- `/api/v1/plataforma/…`: operación de la plataforma; exige **segundo factor** activo.

### Al entrar: elegir la entidad

Una persona entra a **gestionar una PH**, **gestionar una empresa cliente** o **gestionar la
plataforma**, y puede tener varias (la PH donde es propietaria, la empresa donde es contadora…). Lo
primero después de iniciar sesión es **elegir en cuál entrar**:

1. `POST /auth/sesion` (o `POST /auth/sesion/segundo-factor`) devuelve los tokens **y**
   `entidades`: `entidad_id`, `nombre`, `nit`, `tipo`, `estado` y `roles`; primero la plataforma
   y luego por nombre.
   La misma lista sale de `GET /yo/entidades` (para el selector «cambiar de entidad»).
2. Con una sola entidad, entre directo; con varias, muestre el selector; sin ninguna, muestre que
   la cuenta aún no tiene roles. Las entidades **retiradas** no llegan; una `suspendida` o
   `en_configuracion` sí, para avisarlo.
3. El `tipo` decide los menús: **`propiedad_horizontal`** muestra los de la PH (bienes privados,
   censo, asambleas, reservas, cartera… en `/copropiedades/{id}/…`) además de lo común;
   **`empresa`** solo lo común a toda entidad (`/entidades/{id}/…`: contabilidad, facturación…);
   **`plataforma`** la consola de operación (`/plataforma/…`, según `permisos_plataforma` de
   `GET /yo`; exige segundo factor) y lo común de la propia empresa de la plataforma. La
   plataforma aparece a quien tiene roles de plataforma aunque no tenga roles en su empresa (sus
   `roles` traen entonces los de plataforma: `superusuario`, `soporte`…). En una PH los prefijos
   `/copropiedades/{id}` y `/entidades/{id}` usan el **mismo id**.
4. Guarde la entidad elegida en el cliente: la sesión no queda atada a ella y se cambia sin
   volver a iniciar sesión.

Al entrar a una, pida `GET /api/v1/entidades/{id}/capacidades` (en una PH también
`/copropiedades/{id}/capacidades`): devuelve el **tipo**, sus **permisos** efectivos allí
(`Permiso`) y los **módulos** contratados (`Modulo`). Úselos para mostrar u ocultar menús y
botones; no deduzca permisos de los nombres de los roles, porque cada copropiedad los configura.
El backend vuelve a exigirlos: 403 `sin_permiso` o `modulo_no_habilitado`. Vuelva a pedir las
capacidades si recibe uno de esos 403 (pudieron cambiar los roles o el contrato).

### Entidades y contabilidad (ADR 0016, 0017)

Toda empresa que usa la plataforma es una **entidad**; una copropiedad es una entidad con la
vertical PH y **el mismo id**. Lo de cualquier entidad va en `/api/v1/entidades/{id}/…`:
`/capacidades` y la contabilidad (`/contabilidad/…`: configuración, PUC, cuentas, terceros,
periodos, comprobantes, reglas, plantillas, reportes). Una entidad que no es copropiedad
responde 404 en `/copropiedades/{id}/…`. `GET /yo/entidades` lista todas las entidades de la
cuenta con su tipo.

- Los **montos** viajan como texto decimal (`"150000.00"`), nunca como número de punto flotante.
- Un comprobante se guarda en **borrador**, se puede **previsualizar** y se **contabiliza**; ya
  contabilizado no se edita (409): se **revierte**. Las reglas contables incumplidas responden
  422 `regla_contable` con el motivo (partida doble, periodo no abierto, cuenta que no admite la
  línea).
- Los reportes aceptan `formato=csv` (exige `contabilidad.exportar`); la respuesta trae la
  cabecera `X-Contenido-Sha256` y queda en la auditoría.
- **Tesorería** (ADR 0018, etiqueta `tesoreria`): `/contabilidad/tesoreria/…` (cuentas,
  recibos, egresos, extractos y conciliación), `/contabilidad/cuentas-por-pagar/…` y
  `/contabilidad/cuentas-por-cobrar/edades` (cartera de clientes). Las facturas de venta están en
  la facturación (abajo). Los documentos no se editan ni se borran: se **anulan** con
  `POST …/anulacion` y un motivo (se revierte su comprobante). Un egreso pasa por
  `…/autorizacion` (otra cuenta: si es la misma responde 409 `conflicto`) y `…/pago`. El extracto
  bancario viaja en Base64 dentro del JSON (`archivo_base64`) con el mapeo de columnas del banco;
  primero `…/extractos/vista-previa` y luego `…/extractos`. El certificado de retenciones en PDF
  es `GET …/certificados/{persona_id}/pdf`.
- **Cartera de la PH** (ADR 0019, etiqueta `cartera`, módulo `contabilidad_ph`):
  `/copropiedades/{id}/cartera/…`. Orden de configuración: conceptos de cobro → política →
  presupuesto (`…/presupuestos`, luego `…/aprobacion`) → referencia de pago de cada bien. Cada mes:
  `…/liquidaciones/vista-previa` (no guarda) y `POST …/liquidaciones` (409 si el periodo ya está
  liquidado); las cuentas de cobro se descargan en `…/cuentas-cobro/{id}/pdf` y se envían con
  `…/liquidaciones/{id}/envio`. Un pago es `POST …/cartera/recaudos` (se imputa solo; lo que
  sobra queda como `anticipo`); se anula con `…/recaudos/{recibo_id}/anulacion`, no desde la
  tesorería (409). Lo liquidado se corrige con `…/obligaciones/{id}/notas-credito`. Los saldos
  nunca se editan: `…/bienes/{id}/estado-cuenta`, `…/saldos` y `…/cuadre` los calculan.
  Mora: la plataforma carga el interés bancario corriente (`/plataforma/tasas-referencia`); cada
  mes `…/intereses/vista-previa` y `POST …/intereses`. Acuerdos de pago: `POST …/acuerdos` y los
  aprueba **otra cuenta** (`…/acuerdos/{id}/aprobacion`, 409 si es la misma). Edades:
  `…/edades`. Paz y salvo o certificado de deuda: `POST …/bienes/{id}/certificados` devuelve el
  `codigo` y el PDF en Base64 **una sola vez** (guárdelos). La verificación es pública:
  `POST /copropiedades/{id}/certificados/verificacion` con el código (sin sesión, límite por IP).
  Portal del propietario o pagador: `/copropiedades/{id}/mi-cartera/bienes`, `…/estado-cuenta`,
  `…/cuentas-cobro` (y `…/cuentas-cobro/{id}/pdf`) y `POST …/paz-y-salvo` (409 si debe); no
  requiere permisos de cartera, solo ser titular o pagador del bien (404 si no). Informes:
  `…/cartera/informes/fondo`, `…/flujo-caja`, `…/asamblea` (PDF) y
  `POST …/cartera/presupuestos/proyecto`. Morosos de una asamblea desde la cartera:
  `POST …/cartera/asambleas/{reunion_id}/morosos`; restricción de reservas por mora:
  `POST …/cartera/reservas/restricciones-mora` con el umbral.
- **Lo común de la entidad**: `/entidades/{id}/terceros` (clientes y proveedores con sus datos
  fiscales), `/entidades/{id}/tributario/{catalogo,perfiles,resoluciones}`,
  `/entidades/{id}/roles`, `…/asignaciones`, `…/cuentas` (buscar a quién asignar),
  `…/archivos` y `…/auditoria`. Sirven a la PH, a la empresa y a la plataforma; terceros y
  tributario, con la contabilidad **o** la facturación.
- **Perfil tributario** (ADR 0020, etiqueta `tributario`): `POST /entidades/{id}/tributario/perfiles`
  (queda en `borrador`) y `POST …/perfiles/{id}/confirmacion` (lo confirma el contador;
  confirmado no se edita: se crea otra versión). **Sin perfil confirmado** una línea de venta con
  IVA responde 422, no se practican retenciones ni se emiten documentos electrónicos. Resolución
  de numeración: `…/tributario/resoluciones` (la clave técnica nunca vuelve, solo
  `tiene_clave_tecnica`). Preparación de declaraciones (solo con contabilidad, no presenta):
  `GET /entidades/{id}/contabilidad/tributario/retencion|iva|ica?desde=&hasta=`,
  `POST …/contabilidad/tributario/exogena` (devuelve el CSV y su `sha256`) y beneficiarios
  finales en `…/contabilidad/tributario/beneficiarios`.
- **Facturación** (ADR 0022, etiqueta `facturacion`): `/entidades/{id}/facturacion/…`. Sirve con
  el módulo `facturacion_electronica` **sin contabilidad** o con `contabilidad`. Mire los
  `modulos` de `capacidades`: sin `contabilidad` no muestre cuentas ni centros de costo (422 si
  se envían), el pago se marca con `POST …/facturas/{id}/pago` (409 si la entidad lleva
  contabilidad: ahí se paga con un recibo de caja).
  - Catálogo: `…/items` (código único, unidad DIAN, precio sugerido, IVA, clasificación). Una
    línea de factura con `item_id` toma del ítem lo que no traiga (descripción, precio, IVA,
    clasificación y, con contabilidad, la cuenta).
  - Facturas: `POST …/facturas` (borrador) → `…/emision` (número interno del año; con
    contabilidad se contabiliza) → `…/factura-electronica` (repetirla devuelve el mismo documento
    y CUFE; si quedó en `contingencia`, la reintenta). Sin FE, anular es `…/anulacion`; con FE
    validada, `…/nota-credito` (la anulación responde 409). Cada línea lleva `clasificacion`
    (solo `alquiler_servicio` y `venta` admiten IVA).
  - Documento soporte: registrar la compra en `POST …/compras` (sin contabilidad, sin cuentas ni
    retenciones) y luego `POST …/compras/{id}/documento-soporte`.
  - Documentos: `…/documentos`, `…/documentos/{id}`, `…/reintento`, `…/pdf` (con el QR de la
    DIAN en cada página) y `…/paquete` (el ZIP que recibe el adquirente). Al validarse, el
    adquirente recibe ese ZIP por correo en su `correo_facturacion`. Un documento en
    `contingencia` (la DIAN no respondió) se reintenta solo cada 30 minutos; `…/reintento` lo
    fuerza. Sin el motor propio el proveedor es el **simulado** (`proveedor: "simulado"`, sin
    validez ante la DIAN).
  - Facturas recibidas: el XML va en Base64 (`archivo_base64`) a `POST …/recibidas` (409 si ya se
    cargó); luego `…/vinculo`, `…/rechazo` o `…/eventos` (RADIAN en orden: `acuse` →
    `recibo_bien` → `aceptacion` o `reclamo` con motivo; si faltan los previos, 422).
  - Para el contador externo (`facturacion.exportar`): `GET …/libro-ventas`, `…/libro-compras`
    (CSV) y `…/paquete` (ZIP con XML y PDF), todos con `?desde=&hasta=` y la cabecera
    `X-Contenido-Sha256`.
  - Si la entidad habilita la contabilidad después: `POST …/contabilizacion` con el periodo
    contabiliza lo facturado (informe con `contabilizadas`, `pagadas_a_mano` y `fallidas`).
  - **Habilitación con el motor propio** (`facturacion.configurar`; el certificado exige
    `facturacion.certificado.gestionar`): `GET …/habilitacion` (estado, certificado con
    `por_vencer` y envíos del set) → `PUT …/habilitacion/software` (SoftwareID, PIN y TestSetId
    del catálogo de participantes de la DIAN) → `POST …/habilitacion/certificado`
    (`archivo_base64` del `.p12` y `contrasena`) → `POST …/habilitacion/set-pruebas` con la
    `cantidad` que pide la DIAN → `POST …/habilitacion/set-pruebas/consulta` hasta que el estado
    sea `habilitado` → `POST …/habilitacion/produccion` → `POST …/habilitacion/rangos`. El PIN, el
    `.p12` y su contraseña **nunca vuelven** en una respuesta. Sin `FACTURACION_CLAVE_MAESTRA` en el
    servidor, registrar el software responde 422. Para que las facturas salgan por el motor propio,
    el perfil tributario confirmado debe tener `canal_facturacion: "propio"`.

## 4. Perfiles y módulos de la propiedad horizontal

### Perfiles y pantallas de inicio

Los roles son configurables por cada PH: la interfaz se arma con los **permisos** de
`capacidades`, no con el nombre del rol. Estas son las pantallas de inicio que tienen sentido
según lo que la cuenta puede hacer (un mismo usuario puede tener varios perfiles en entidades
distintas):

| Perfil típico | Se reconoce por | Pantalla de inicio sugerida |
|---|---|---|
| Administración de la PH | `copropiedad.datos.editar`, `tickets.ver_todos`, `comunicados.gestionar`… | Tablero: tickets (`…/tickets/tablero`), alertas del censo (`…/censo/alertas`), mantenimiento (`…/mantenimiento/alertas`), próximas asambleas y cartera. |
| Contador o revisor | `contabilidad.ver`, `tributario.preparar` | Periodos, comprobantes en borrador, conciliaciones pendientes, perfil tributario. |
| Facturador (empresa sin contabilidad) | `facturacion.facturas.gestionar` | Facturas del mes, documentos en `contingencia` o `rechazado`, catálogo. |
| Portería | `acceso.registrar` | Pantalla de turno: anunciar visita, ingreso con QR, correspondencia, minuta. Pensada para tableta, botones grandes. |
| Mesa de la asamblea | `asambleas.registrar` | Búsqueda en el padrón (`…/mesa`), registro de asistentes, rótulos. |
| Propietario o residente | Rutas `mis-…` y permisos de autoservicio | «Mis bienes», mi cartera, mis paquetes y visitas, reservas, mis tickets, comunicados, papeleta. Diseño móvil primero. |
| Operador de la plataforma | `permisos_plataforma` de `GET /yo` (exige segundo factor) | Consola: entidades y módulos, moderación, catálogos normativos, líneas de WhatsApp, consumo. |
| Anunciante | `GET /anunciantes` con resultados | Portal: ficha, campañas, métricas, leads, redimir cupones. |

**Rutas del propio usuario** (no exigen permisos de administración, solo el vínculo):
`/yo/…` (perfil, correo, contraseña, segundo factor, sesiones, dispositivos, `GET /yo/tickets`
de todas sus PH), y en cada PH `…/mis-bienes-privados`, `…/mi-cartera/…`, `…/mis-comunicados`,
`…/proteccion-datos/mis-datos`, `…/reservas?mias=true`, `…/cambios-censo?mias=true`, visitas y
correspondencia (un residente solo ve las de su bien).

### Puesta en marcha de una PH (orden recomendado del asistente de configuración)

1. Ficha (`PATCH /copropiedades/{id}`: datos legales, ubicación, `zona_horaria`) y puntos de
   atención con horario.
2. Bienes privados: a mano (agrupaciones → bienes) o, mejor, con la **importación**:
   `GET …/importaciones/bienes-privados/plantilla` → `POST …/importaciones/bienes-privados`
   (archivo crudo; devuelve la **vista previa** con errores por fila, sin aplicar nada) →
   `POST …/importaciones/{id}/confirmar`. Muestre los errores en una tabla por fila y columna
   (el mensaje no repite el valor de la celda).
3. **Coeficientes**: nunca se editan uno por uno. Cuadro borrador (`POST …/cuadros-coeficientes`,
   puede copiar el vigente) → `PUT …/coeficientes` (devuelve la suma y la diferencia) →
   `…/validar` → `…/activar`. Si no suman **exactamente** 1: 422 `coeficientes_no_suman_uno` con
   la diferencia en el mensaje; muéstrela junto al total. No se activa con una asamblea instalada.
4. Titulares de cada bien (suman 100 % por clase de derecho y uno es el representante); cambiar de
   dueño es `…/transferencias`, no editar. El rol `propietario` se deriva solo.
5. Roles y asignaciones (`/entidades/{id}/roles`, `…/asignaciones`; buscar la cuenta con
   `…/cuentas?identificador=` con el usuario o el correo exactos; no hay listados de cuentas). Nadie otorga lo que no tiene: un 403 al asignar
   significa eso.
6. Censo (residencias, vehículos, mascotas) y política de protección de datos.
7. Módulos contratados según `capacidades.modulos`.

### Censo (`censo`)
Residencias con su calidad (`CalidadResidencia`), vehículos (placa única entre los vigentes) y
mascotas. Lo que propone un residente o propietario entra a `…/cambios-censo` y solo se aplica al
aprobarlo (`censo.aprobar`): muestre la bandeja de propuestas con aprobar/rechazar (motivo
obligatorio). Libros de propietarios y residentes a una fecha de corte (`…/libros/…`, JSON, CSV o
PDF). Campañas de actualización por WhatsApp: `…/censo/campanas` y sus respuestas por atender.
Fecha de nacimiento y movilidad reducida son **datos sensibles**: llegan ocultos sin
`personas.datos_sensibles.ver`; no los pida en formularios a quien no puede verlos.

### Bienes comunes y reservas (`reservas`, módulo `bienes_comunes`)
Todo lo reservable es un **recurso** (`TipoRecurso`: bien común, parqueadero, agenda de la
administración, herramienta) con su política. Calendario: `…/recursos/{id}/disponibilidad` da las
franjas libres (horario − excepciones − reservas − alistamiento), en hora local de la PH.
`POST …/reservas` responde `aprobada` (confirmación inmediata) o `solicitada`. Ciclo:
solicitada → aprobada → en_curso (`…/entrega`) → finalizada o con_novedad (`…/recibo`), o
cancelada, rechazada, no_asistio. Cancelar tarde puede generar penalidad. Un bien en mora o
sancionado no reserva (`…/restricciones-reserva`). Parqueaderos y sorteos con semilla publicada
(muestre el compromiso al crear y la semilla al ejecutar: es la prueba de transparencia).

### Tickets y chat (`tickets`, `conversaciones`, módulo `gestion_administrativa`)
Número `PQR-AAAA-NNNNN`, vencimiento calculado en días u horas hábiles (muestre el semáforo con
`vence_en` y `vencido`). Estados en `EstadoTicket`; las transiciones permitidas dependen del papel de la cuenta
(gestiona, atiende o es el solicitante): si una transición no aplica, 409 `conflicto_estado`.
Mensajes públicos y **notas internas** (el solicitante nunca las ve; márquelas distinto). Los
reportes de convivencia ocultan la identidad sin `tickets.convivencia.ver`. Encuesta de
satisfacción una vez resuelto. La bandeja de **conversaciones** son los chats de WhatsApp que el
bot escaló: responder solo dentro de las 24 h desde el último mensaje del residente (fuera de esa
ventana o con la conversación cerrada, 409 `conflicto`; avíselo antes de que escriba).

### Comunicados (`comunicados`, módulo `gestion_administrativa`)
Borrador → programado o enviado (o anulado). Al crear se elige la **audiencia** (`TipoAudiencia`)
y la **finalidad**: `comunicaciones` solo llega a quien la autorizó (Ley 1581). Salen por push y
correo; por WhatsApp solo si es importante y la persona no tiene la app. El detalle trae el
tablero de entrega y lectura. Residente: `…/mis-comunicados` (abrir registra la lectura y entrega
enlaces de adjuntos de 5 min) y «Enterado».

### Mantenimiento (`mantenimiento`, módulo `gestion_administrativa`)
Activos con hoja de vida (archivos, movimientos, órdenes); la baja es definitiva y exige soporte.
Planes preventivos que generan órdenes solas; órdenes: programada → en_ejecucion → ejecutada →
verificada (**quien ejecuta no verifica**: oculte el botón a esa misma cuenta). Obligaciones
periódicas desde un catálogo sugerido y su historial de cumplimientos. Tablero y alertas.

### Asambleas y votaciones (`asambleas`, módulo `asambleas`)
Es el módulo con más reglas legales; siga este orden de pantallas:

1. **Preparación**: crear la reunión (`TipoReunion`, modalidad, configuración: poderes por
   apoderado, `EfectoMora`, `BaseComputo`, temporizador) → orden del día (`…/puntos`) → anexos →
   preguntas en borrador (`TipoPregunta`, `TipoVoto` nominal o reservado).
2. **Convocatoria** (`…/convocatoria`): fija el **padrón** y ya no cambia; envía WhatsApp y correo.
   Constancias de envío con su acuse en `…/convocatoria/constancias`. Morosos: `PUT …/morosos` o
   planilla, después de convocar y antes de instalar.
3. **Registro** (`…/transiciones` a `en_registro`): la mesa busca en `…/mesa` por código de bien o
   documento, registra al asistente con sus bienes y poderes en un toque; cuenta para el quórum de
   inmediato. Rótulos Code 128 (`POST …/asambleas/rotulos`) para votar levantando el rótulo.
4. **Instalación** (exige quórum) y conducción: `GET …/quorum`, tablero en vivo por WebSocket
   (§13), puntos que avanzan, salidas y reingresos. Sin quórum en primera:
   `…/segunda-convocatoria`.
5. **Votación**: abrir la pregunta (habilita a los presentes y les llega por WhatsApp) → votos por
   web (papeleta), WhatsApp, presencial o rótulo → cerrar (guarda el escrutinio). El asistente
   vota **una vez por todos sus bienes**; un segundo intento da 409 `ya_voto` (y una pregunta ya cerrada,
   409 `conflicto_estado`); si el asistente no está presente o no representa el bien, 403
   `no_inscrito`. El escrutinio muestra coeficiente **y** unidades. En preguntas
   **reservadas** los resultados públicos son totales; el detalle exige
   `asambleas.votos.ver_detalle` y cada consulta queda auditada (adviértalo en la interfaz).
6. **Cierre y acta**: `GET …/acta` (revisar) → `POST …/acta` (PDF) → `PUT …/acta/estado`
   (escaneo firmado, aprobada, publicada). Elecciones de consejo o revisor: `…/eleccion` crea los
   roles. Reportes de asistencia, votos, canales y costo de WhatsApp.

**Papeleta virtual** (sin sesión, cabecera `X-Papeleta`, §12): `GET …/papeleta`, ingreso, salida y
`POST …/papeleta/votos` con `Idempotency-Key` **obligatoria**. Diseño móvil, una pregunta a la vez,
confirmación clara del voto registrado.

### Control de acceso (`acceso`, módulo `control_acceso`)
- **Portería**: anunciar visita (`POST …/visitas`) → el backend avisa al residente responsable
  (push primero, WhatsApp de respaldo), a los 2 min a todos los residentes y a los 5 min pasa a
  `requiere_llamada`: entonces y solo entonces `…/contactos` da los teléfonos (cada consulta se
  audita) y se registra `…/llamada`. La primera decisión gana. Ingreso, salida, cancelación.
  Refresque la lista de visitas abiertas con frecuencia (o al recibir el push de portería).
- **Residente**: autorizar o rechazar (`…/respuesta`), pre-autorizar y compartir el QR
  (`…/preautorizaciones`, `…/qr`), ver sus paquetes.
- **Correspondencia**: registrar → avisos cada 24 h → entrega con **firma o foto** (en Base64 dentro
  del JSON) a una persona de `…/personas-para-entrega`, o devolución.
- Minuta (append-only), bitácora de eventos, puntos de acceso, puentes y dispositivos,
  credenciales (`TipoCredencial`, nunca vuelve el valor en claro), autorizaciones manuales y
  consentimiento biométrico (facultativo, siempre con método alterno).

### Protección de datos (`proteccion_datos`)
Política versionada; la persona autoriza por **finalidad** bajo la política vigente y puede
revocar. Muestre la política antes de pedir la autorización. Solicitudes del titular (consulta,
reclamo…) con vencimiento en días hábiles y una prórroga única: muestre el plazo restante.
«Mis datos» (`…/proteccion-datos/mis-datos`) para el titular.

### Directorio y publicidad (`directorio`, `publicidad`)
El directorio funciona en toda PH; las fichas destacadas solo con el módulo `publicidad`.
Reseñas anónimas (una por cuenta, solo propietarios y residentes vigentes) y reporte para
moderación. «Busco …»: `POST …/directorio/busquedas` y **solo** con el «Sí» explícito
(`…/contacto`) se comparte el contacto con esos proveedores: haga ese consentimiento visible y
específico. Piezas publicitarias solo para quien autorizó la publicidad de la plataforma
(`…/publicidad/autorizacion`; la PH no puede autorizar por el titular). Pantalla del lobby con
`X-Pantalla`. Portal del anunciante en `/anunciantes/…` (campañas y creativos siempre pasan por
moderación).

### Organizaciones y contratos (`organizaciones`)
Administradoras, contables, vigilancia… son organizaciones globales que se buscan por **NIT
exacto** antes de crearlas (para no duplicarlas). Un contrato de servicio vincula la organización
con la PH y su personal recibe roles **derivados** que vencen con el contrato.

### Consola de la plataforma (`plataforma`)
Exige segundo factor (si falta: 403 `segundo_factor_requerido`, lleve a `/yo/segundo-factor`).
Alta de copropiedades y entidades, estado y **módulos** de cada una, roles de plataforma, bloqueo
de cuentas, reclamaciones de organizaciones, líneas y tarifas de WhatsApp, catálogos normativos
(PUC, retenciones y UVT, IVA, ICA, exógena, tasas de referencia), plantillas contables, consumo de
facturación electrónica, moderación del directorio y la publicidad.

## 5. Estados de los recursos

Cada recurso con ciclo de vida trae `estado` con un enum con nombre en el contrato. Las acciones
que no aplican al estado actual responden 409 `conflicto_estado`: **no muestre esos botones** y,
si aun así llega el 409, recargue el recurso. Los principales:

| Recurso | Estados (`enum`) | Notas para la interfaz |
|---|---|---|
| Reunión | `EstadoReunion`: borrador → convocada → en_registro → instalada ⇄ suspendida → cerrada; o cancelada | Tras convocar, el padrón es fijo. |
| Pregunta | `EstadoPregunta`: borrador → abierta → cerrada; o anulada | Solo abiertas de reuniones instaladas reciben votos. |
| Punto del orden del día | `EstadoPunto`: pendiente → en_curso → terminado; o aplazado | |
| Acta | `EstadoActa`: pendiente → generada → aprobada → publicada | Regenerable hasta aprobarse. |
| Poder | `EstadoPoder`: pendiente, verificado, rechazado, revocado | |
| Ticket | `EstadoTicket` (11 valores) | Transiciones según el papel; `reabierto` dentro del plazo de la configuración. |
| Reserva | `EstadoReserva`: solicitada, aprobada, rechazada, en_curso, finalizada, con_novedad, cancelada, no_asistio | Pago aparte: `EstadoPago`. |
| Visita | `EstadoVisita`: anunciada, requiere_llamada, autorizada, rechazada, sin_respuesta, ingresada, finalizada, vencida, cancelada | `requiere_llamada` = resaltar en portería. |
| Correspondencia | `EstadoCorrespondencia`: recibida → entregada o devuelta | |
| Credencial | `EstadoCredencial`: activa, suspendida, revocada, perdida | |
| Comunicado | `EstadoComunicado`: borrador → programado → enviado; o anulado | Lo enviado no se anula. |
| Orden de trabajo | `EstadoOrden`: programada → en_ejecucion → ejecutada → verificada; o cancelada | |
| Activo | `EstadoActivo`: operativo, con_falla, en_reparacion, fuera_de_servicio, dado_de_baja | La baja es definitiva. |
| Cambio de censo | `EstadoCambioCenso`: pendiente → aprobado o rechazado | |
| Cuadro de coeficientes | `EstadoCuadro`: borrador → vigente → historico | |
| Importación | `EstadoImportacion`: validada, con_errores, aplicada | Solo `validada` se confirma. |
| Comprobante contable | `EstadoComprobante`: borrador → contabilizado | Contabilizado no se edita: se revierte. |
| Periodo contable | `EstadoPeriodo`: abierto, en_revision, cerrado | Reabrir lo aprueba otra cuenta (`EstadoReapertura`). |
| Egreso | `EstadoEgreso`: por_autorizar → autorizado → pagado; o anulado | Autoriza otra cuenta. |
| Factura de venta | `EstadoFacturaVenta`: borrador → emitida → pagada_parcial → pagada; o anulada | |
| Factura de proveedor | `EstadoFacturaProveedor`: registrada, causada, pagada_parcial, pagada, anulada | |
| Documento electrónico | `EstadoDocumentoElectronico`: por_enviar, enviado, validado, rechazado, contingencia | Un rechazado nunca se valida; `contingencia` se reintenta sola. |
| Habilitación DIAN | `EstadoHabilitacion`: registrado → en_pruebas → habilitado → produccion | Úselo como asistente paso a paso. |
| Perfil tributario | `EstadoPerfilTributario`: borrador → confirmado | Confirmado no se edita: nueva versión. |
| Acuerdo de pago (cartera) | `EstadoAcuerdo`: propuesto, vigente, cumplido, incumplido, anulado | Lo aprueba otra cuenta. |
| Solicitud del titular | `EstadoSolicitudTitular`: radicada → en_tramite → respondida | |
| Campaña publicitaria | `EstadoCampana`: borrador, en_revision, aprobada, rechazada, activa, pausada, finalizada | Editar una aprobada la devuelve a revisión. |
| Entidad / copropiedad | `EstadoCopropiedad`: en_configuracion, activa, suspendida, retirada | Avise si la entidad no está activa. |

## 6. Sesión

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

## 7. Errores

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

Catálogo completo (`CodigoError`) y qué hacer en la interfaz:

| `codigo` | HTTP | Qué significa | Qué hacer en la interfaz |
|---|---|---|---|
| `peticion_invalida` | 400 | JSON o parámetro mal formado. | Es un error del cliente: registrarlo; al usuario, un mensaje genérico. |
| `no_autenticado` | 401 | Falta la sesión o venció. | Renovar una vez (§6); si falla, ir al inicio de sesión. |
| `credenciales_invalidas` | 401 | Usuario, contraseña o código incorrectos. | Mensaje en el formulario, sin decir cuál de los datos falló. |
| `sin_permiso` | 403 | La cuenta no tiene el permiso. | Volver a pedir `capacidades` y ocultar la acción. |
| `segundo_factor_requerido` | 403 | La plataforma exige segundo factor. | Llevar a activar el TOTP (`/yo/segundo-factor`). |
| `no_inscrito` | 403 | El asistente no está presente o no representa el bien. | En la papeleta: invitar a registrar el ingreso o acudir a la mesa. |
| `enlace_invalido` | 403 | Enlace firmado vencido o alterado. | Pedir un enlace nuevo (§11) y abrirlo de una vez. |
| `modulo_no_habilitado` | 403 | La entidad no contrató el módulo. | Ocultar el menú; si llega, pantalla «Módulo no contratado» con contacto comercial. |
| `no_encontrado` | 404 | No existe o la cuenta no tiene roles allí. | Pantalla de «no encontrado»; no insinuar que existe. |
| `conflicto` | 409 | Ya existe o rompe una regla (último administrador, misma cuenta que aprueba…). | Mostrar `mensaje` junto a la acción. |
| `conflicto_estado` | 409 | La acción no aplica en el estado actual. | Recargar el recurso y repintar los botones. |
| `ya_voto` | 409 | Los bienes ya votaron esa pregunta. | Mostrar «Su voto ya quedó registrado» (no es un fallo). |
| `validacion` | 422 | Datos bien formados que rompen una regla de negocio. | Mostrar `mensaje` cerca del formulario. |
| `idempotencia_reusada` | 422 | La `Idempotency-Key` se usó con otro cuerpo. | Error del cliente: generar una clave nueva por acción (§8). |
| `coeficientes_no_suman_uno` | 422 | El cuadro no suma exactamente 1. | Mostrar la diferencia del mensaje junto al total. |
| `regla_contable` | 422 | Partida doble, periodo no abierto, cuenta que no admite la línea… | Mostrar el motivo en el comprobante. |
| `demasiados_intentos` | 429 | Muchos intentos fallidos de inicio de sesión o código. | Bloquear el formulario unos minutos. |
| `demasiadas_peticiones` | 429 | Límite por IP de la ruta. | Esperar `Retry-After` (§10). |
| `interno` | 500 | Error inesperado. | Mensaje genérico con el `X-Request-Id` para soporte. |
| `no_disponible` | 503 | Una dependencia no responde. | Reintentar más tarde con espera creciente. |

## 8. Reintentos (`Idempotency-Key`)

Los `POST` con sesión aceptan `Idempotency-Key` (1 a 200 caracteres). Genere un UUID por
**acción del usuario** (no por intento) y reutilícelo si la red falla y reintenta: durante 24 h
la API devuelve la respuesta original con `Idempotency-Replayed: true`, sin duplicar nada. La
misma clave con otro cuerpo responde 422 `idempotencia_reusada`. En el voto presencial la
cabecera es obligatoria.

## 9. Listados paginados

`?limite=50&despues=<cursor>` (`limite` de 1 a 200, 50 por defecto). La respuesta
(`PaginaRespuesta_…`) trae `datos` y `siguiente`: pase `siguiente` como `despues` para la página
siguiente; `null` indica que es la última. No hay números de página ni totales (por cursor).

## 10. Límites de peticiones

Las rutas sin sesión (inicio de sesión, recuperación, papeleta, enlaces públicos) tienen límite
por IP. Al superarlo: 429 `demasiadas_peticiones` con `Retry-After` (segundos). Espere ese tiempo
antes de reintentar y muestre un aviso; no reintente en bucle.

## 11. Archivos

- **Subir**: `POST /api/v1/entidades/{id}/archivos?nombre=…&clasificacion=…` con el contenido
  crudo en el cuerpo (`Content-Type: application/octet-stream`, no `multipart`). El tipo real se
  valida por contenido; un tipo no admitido o muy grande da 422 `validacion`. La respuesta trae
  el `id` del archivo, que luego se adjunta en otras peticiones (tickets, reservas, actas…).
- **Descargar**: pida `GET …/archivos/{archivo_id}/enlace` y abra la URL que devuelve: es un enlace
  firmado de **5 minutos** que no necesita sesión. Pídalo justo antes de abrirlo; no lo guarde.

## 12. Credenciales sin sesión de persona

| Cabecera | Para qué |
|---|---|
| `X-Papeleta` | Papeleta virtual de una asamblea (el token del enlace personal del asistente). |
| `X-Enlace-Directorio` | Directorio público de la copropiedad (enlace que se comparte en grupos). |
| `X-Pantalla` | Pantalla de publicidad del lobby. |

El token llega en el enlace que se le entrega a la persona; el frontend lo lee de la URL y lo
envía en la cabecera, nunca en la ruta de la API.

## 13. Tablero en vivo de las asambleas (WebSocket)

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

## 14. Notificaciones push (apps)

Registre el token de Firebase con `POST /api/v1/yo/dispositivos-push` al iniciar sesión y cuando
Firebase lo renueve; elimínelo con `DELETE /api/v1/yo/dispositivos-push/{dispositivo_id}` al
cerrar sesión. Un token inválido se desactiva solo y el aviso se envía por WhatsApp.

## 15. Sugerencias de diseño para aprovechar el backend

- **Permisos y módulos primero.** Cargue `capacidades` al entrar a la entidad y úselas para menús,
  botones y rutas del enrutador. Un 403 no debería verse en el uso normal.
- **Botones según el estado** (§5): el backend rechaza lo que no aplica; la interfaz no debería
  ofrecerlo. Tras cada acción use la respuesta (trae el recurso actualizado) en vez de volver a
  pedir la lista.
- **Doble clic y redes móviles**: genere una `Idempotency-Key` por acción en todo `POST` (pagos,
  recaudos, votos, emisiones, radicaciones) y deshabilite el botón mientras espera.
- **Descargas** (PDF, CSV, ZIP): las rutas con sesión exigen `Authorization`, así que no sirven
  como `<a href>`. Pídalas con `fetch`, cree un `Blob` y use el nombre de `Content-Disposition`.
  Los archivos del almacén van por enlace firmado (§11). Si viene `X-Contenido-Sha256`, muéstrelo
  en exportaciones para el contador: es la huella que quedó en la auditoría.
- **Vista previa antes de aplicar**: importaciones, liquidaciones, intereses, extractos y
  comprobantes tienen una ruta de vista previa que no guarda nada. Diseñe esos flujos en dos pasos
  (revisar → confirmar).
- **Lo que no se edita se corrige con otra acción**: comprobantes (revertir), recaudos y egresos
  (anular), facturas validadas (nota crédito), coeficientes (cuadro nuevo), titulares
  (transferencia). No ponga «Editar» en esos recursos: ponga la acción correcta con su motivo.
- **Cuatro ojos**: egresos, acuerdos de pago y reapertura de periodos los aprueba **otra cuenta**.
  Oculte «Aprobar» a quien creó el registro.
- **Fechas en la zona de la PH** y montos con una librería decimal (§2). Las monedas se muestran
  con separador de miles colombiano y dos decimales.
- **Búsquedas sin tildes**: el backend ya normaliza (mayúsculas y tildes); no hace falta hacerlo
  en el cliente. Use un retardo de ~300 ms al escribir.
- **Listas largas**: paginación por cursor (§9), ideal para desplazamiento infinito; no hay total,
  así que no muestre «página X de Y».
- **Tiempo real**: el tablero de la asamblea va por WebSocket (§13). Para lo demás (visitas en
  portería, documentos en contingencia) basta refrescar cada 15–30 s o al volver a la pestaña.
- **Datos personales** (Ley 1581): no guarde en el navegador datos de personas más allá de la
  sesión, no los ponga en la URL ni en analítica, y muestre la política antes de pedir una
  autorización. Los datos sensibles llegan ocultos si la cuenta no puede verlos: muestre «oculto»,
  no un campo vacío editable.
- **Secretos que no vuelven**: tokens de sesión, PIN y certificado de facturación, clave técnica,
  token de un puente o de la pantalla, código de un certificado de cartera. El backend los muestra
  **una sola vez**: diseñe una pantalla de «cópielo ahora» con botón de copiar y advertencia.
- **Auditoría visible**: las consultas sensibles (detalle de votos reservados, contactos de una
  visita, datos tratados de una persona) quedan registradas. Avíselo antes de abrirlas.
- **Accesibilidad y contexto**: portería y mesa de asamblea se usan de pie, en tableta y con prisa
  (botones grandes, alto contraste, pocas pantallas); residentes, desde el celular; contabilidad y
  facturación, en escritorio con tablas densas y atajos de teclado.
- **Errores con contexto**: muestre el `mensaje` del backend (ya es seguro y en español) y, en los
  inesperados, el `X-Request-Id` con un botón de copiar.

## 16. Cuando cambia el contrato

Cada PR del backend que cambia la API trae el `openapi.json` regenerado y el índice
[guia-frontend-endpoints.md](guia-frontend-endpoints.md). Regenere el cliente, compile y revise
los errores de tipos: un cambio incompatible (campo eliminado o renombrado, valor de enum
retirado) aparece ahí antes de llegar a producción.
