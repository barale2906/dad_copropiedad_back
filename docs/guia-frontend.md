# Guía de integración para el frontend

Qué necesita saber la web (Vue) y las apps (Flutter) para consumir la API. El **contrato** es
`apps/api/openapi.json` (OpenAPI 3.1): describe cada ruta, parámetro, cuerpo, respuesta, error y
esquema. Esta guía explica lo que el contrato no cuenta por sí solo: los flujos y las
convenciones.

**Cómo usar esta guía**

| Necesito… | Dónde |
|---|---|
| Generar el cliente tipado y conocer los formatos | §1 y §2 |
| **El menú: zonas de la maqueta, módulos que se encienden y acciones por permiso** | **§4** |
| Qué ve cada tipo de usuario y en qué entidad está | §3 (entrar a una entidad) y §5 (perfiles) |
| El flujo de un módulo (asambleas, censo, cartera, facturación, nómina…) | §3 (entidad, contabilidad, facturación, nómina) y §5 (PH) |
| Qué botones mostrar según el estado de un recurso | §6 |
| Sesión, errores, reintentos, paginación, archivos, tiempo real | §7 a §15 |
| Consejos de diseño que aprovechan lo que hace el backend | §16 |
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

Cada operación tiene un `operationId` **único**: la etiqueta del módulo más el nombre del
handler (`tickets_listar`, `reservas_cancelar`, `asambleas_votar_papeleta`). Los generadores que
agrupan tipos por `operationId` ya pueden usarlo. Los campos que solo vienen en las respuestas
(p. ej. el `id` de `PuntoDto` y `DispositivoDto`) se marcan `readOnly`: no hace falta enviarlos.

## 2. Convenciones de datos

| Tipo | Formato JSON |
|---|---|
| Identificadores | UUID en texto (`"0193…"`). |
| Fechas y horas | ISO 8601 en **UTC** (`2026-11-12T15:04:05Z`). Mostrar en la zona de la copropiedad (`zona_horaria`, IANA, p. ej. `America/Bogota`), no en la del navegador. |
| Fechas sin hora | `AAAA-MM-DD`. Horas del día: `HH:MM:SS` (hora local de la copropiedad). |
| Montos y coeficientes | **Texto decimal** (`"0.00523000"`, `"150000.00"`). No convertir a `number` para calcular: se pierde precisión. Mostrar con formato, operar con una librería decimal. |
| Opcionales | `null` o ausentes. |

## 3. Rutas

- `/api/v1/auth/…`: sesión, recuperación y definición de la contraseña (sin sesión). El registro
  abierto (`POST /auth/registro`) solo existe en desarrollo (`REGISTRO_ABIERTO`); en producción
  responde 404 (ver «Usuarios» en el §7).
- `/api/v1/yo/…`: la cuenta autenticada (sus entidades, perfil y permisos de plataforma, sesiones,
  dispositivos push).
- `/api/v1/entidades/{entidad_id}/…`: lo común de cualquier entidad (capacidades, roles,
  archivos, auditoría, terceros, tributario, contabilidad y facturación). Mismo 404 sin roles.
- `/api/v1/copropiedades/{copropiedad_id}/…`: lo propio de una copropiedad. Responde **404** si la
  cuenta no tiene roles en ella (no se revela si existe).
- `/api/v1/organizaciones/{organizacion_id}/…`: portal de las organizaciones. No hay portal del
  anunciante: la publicidad la opera la plataforma (ADR 0023); el comercio solo redime cupones
  con su enlace (`POST /api/v1/publicidad/cupones/redimir` con `X-Comercio`).
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
3. El `tipo` decide la maqueta y los menús (detalle completo en §4): **`propiedad_horizontal`** muestra los de la PH (bienes privados,
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
(`Permiso`), los **módulos** contratados (`Modulo`) y la **zona horaria** de la entidad
(`zona_horaria`, para mostrar las fechas; la reciben todas las cuentas con un rol). Úselos para mostrar u ocultar menús y
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
  primero `…/extractos/vista-previa` y luego `…/extractos`. El último mapeo de la cuenta se
  guarda al importar: prellene el formulario con `GET …/cuentas/{id}/mapeo-extracto` (`PUT` lo
  cambia). El certificado de retenciones en PDF
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
  aprueba **otra cuenta** (`…/acuerdos/{id}/aprobacion`, 409 si es la misma: oculte «Aprobar»
  cuando `propuesto_por` es la cuenta de la sesión). Edades:
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
  fiscales; `…/terceros/{persona_id}` da el nombre y el documento del `persona_id` que traen las
  líneas de un comprobante, los recibos, los egresos y las facturas),
  `/entidades/{id}/tributario/{catalogo,perfiles,resoluciones}`,
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
    retenciones) y luego `POST …/compras/{id}/documento-soporte` (necesita una resolución
    `documento_soporte` y la dirección del vendedor). Se anula con `POST …/compras/{id}/nota-ajuste`
    y un motivo: emite la nota de ajuste y anula la compra.
  - Las notas (crédito, débito, de ajuste) llevan número propio: `prefijo` del tipo y del año
    (`NC26`, `ND26`, `NA26`) y `numero` consecutivo; muéstrelos juntos como en la factura.
  - **Nota débito** (cobros que se agregan a una factura con factura electrónica validada:
    intereses, gastos, cambio del valor): `POST …/facturas/{id}/nota-debito` con `concepto`
    (`ConceptoNotaDebito`), `motivo`, `fecha`, `vencimiento` y `lineas` (como las de una factura).
    Devuelve el documento electrónico; la nota misma aparece en `GET …/facturas` con
    `nota_debito_de` (la factura que ajusta) y se cobra como cualquier factura: entra a la cartera
    por edades, se abona con recibos de caja o, sin contabilidad, se marca pagada a mano. Muestre
    las notas débito debajo de su factura.
  - **Contingencia del facturador** (no puede facturar electrónicamente): registre la resolución
    de su talonario (`POST /tributario/resoluciones` con `tipo: contingencia`), expida en papel y,
    al superar el inconveniente (máximo 48 horas), transcriba cada factura de venta con
    `POST …/facturas/{id}/factura-contingencia` y el `numero_talonario`. 422 si el número no
    está en una resolución de contingencia vigente; 409 si la factura ya tiene otra factura
    electrónica.
  - Documentos: `…/documentos`, `…/documentos/{id}`, `…/reintento`, `…/pdf` (con el QR de la
    DIAN en cada página) y `…/paquete` (el ZIP que recibe el adquirente). Al validarse, el
    adquirente recibe ese ZIP por correo en su `correo_facturacion`. Un documento en
    `contingencia` (la DIAN no respondió) se reintenta solo cada 30 minutos; `…/reintento` lo
    fuerza. Sin el motor propio el proveedor es el **simulado** (`proveedor: "simulado"`, sin
    validez ante la DIAN).
  - Facturas recibidas: el XML va en Base64 (`archivo_base64`) a `POST …/recibidas` (409 si ya se
    cargó); luego `…/vinculo`, `…/rechazo` o `…/eventos`. **Eventos RADIAN** en orden: `acuse` →
    `recibo_bien` → `aceptacion` o `reclamo` (si faltan los previos, 422). El acuse y el recibo
    del bien exigen `quien_recibe` (tipo y número de documento, nombres, apellidos; cargo y área
    opcionales: Código de Comercio, arts. 773 y 774); el reclamo exige `concepto_reclamo`
    (`ConceptoReclamo`) y admite `motivo`. La respuesta trae `numero`, `cude` y el
    `xml_archivo_id` del evento firmado; mire `estado`: la DIAN puede rechazarlo. Diseñe un
    asistente con los cuatro pasos y deshabilite los que aún no aplican.
  - **Aceptación tácita** de una factura propia a crédito, cuando el cliente no la aceptó ni la
    reclamó en los tres días hábiles siguientes al recibo del bien:
    `POST …/documentos/{id}/aceptacion-tacita` (sin cuerpo; la DIAN verifica el plazo). Los
    eventos de un documento propio: `GET …/documentos/{id}/eventos`.
  - El correo al cliente sale al buzón que él registró ante la DIAN como facturador; si no está
    registrado, al `correo_facturacion` del tercero.
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
    `cantidad` de facturas y las `notas_credito` y `notas_debito` que pide la DIAN → `POST …/habilitacion/set-pruebas/consulta` hasta que el estado
    sea `habilitado` → `POST …/habilitacion/produccion` → `POST …/habilitacion/rangos`. El PIN, el
    `.p12` y su contraseña **nunca vuelven** en una respuesta. Sin `FACTURACION_CLAVE_MAESTRA` en el
    servidor, registrar el software responde 422. Para que las facturas salgan por el motor propio,
    el perfil tributario confirmado debe tener `canal_facturacion: "propio"`.
- **Nómina** (ADR 0025, etiqueta `nomina`, módulo `nomina`): `/entidades/{id}/nomina/…`. Se vende
  sola: cualquier entidad (PH o empresa) la contrata sin la contabilidad.
  - Catálogo: `GET …/catalogo?fecha=` da la versión vigente de cada parámetro (con su `unidad`) y
    las administradoras activas. Úselo para llenar los selectores de EPS, pensiones, cesantías,
    caja y ARL (`tipo`). Si falta un parámetro vigente, contratar responde 422 nombrándolo: avise
    que la plataforma aún no cargó el catálogo de esa fecha.
  - Empleados: `GET …/empleados?q=&estado=` (paginado; `estado` = `activo` | `retirado`, derivado
    de los contratos) y `GET …/empleados/{id}` (la ficha con todos los contratos y versiones).
    `POST …/empleados` crea el empleado **con su primer contrato**; mande `persona_id` (alguien que
    ya está en la entidad) **o** `persona` (sus datos; si el documento ya existe se usa esa
    persona). 409 si el código se repite o la persona ya es empleada.
  - Contrato: `tipo` define las fechas (término fijo y aprendizaje llevan `fecha_fin`; obra o
    labor, `descripcion_obra`). Las `condiciones` son una versión: el aprendiz lleva
    `tipo_salario: apoyo_sostenimiento` y su `etapa_aprendizaje`; la jornada parcial,
    `horas_semanales`; la transferencia, la `cuenta`; la ARL, su `clase_riesgo`. Las reglas del
    salario mínimo y de las afiliaciones responden 422 (`validacion`) con el motivo.
  - Cambios: `POST …/contratos/{id}/versiones` (desde una fecha posterior a la última versión;
    muestre el historial de versiones como línea de tiempo), `…/prorroga` (solo término fijo),
    `…/terminacion` (fecha y `motivo`; no antes de la última versión). Un contrato terminado no
    acepta cambios (409). Otro contrato del mismo empleado: `POST …/empleados/{id}/contratos` (409
    si se cruza en fechas).
  - **Salarios y cuentas** solo llegan con `nomina.salarios.ver`; sin él vienen en `null`: oculte
    esas columnas en lugar de mostrarlas vacías.
  - Carga masiva: `GET …/importaciones/plantilla` (CSV), `POST …/importaciones?nombre=` (el archivo
    en el cuerpo) y `…/importaciones/{id}/confirmar`, como las demás importaciones. Las
    administradoras van por su **código PILA**.
  - En la consola de la plataforma (`plataforma.normativa.gestionar`): `GET /plataforma/nomina/
    catalogo`, `POST /plataforma/nomina/parametros` (versión nueva; nunca se edita),
    `POST /plataforma/nomina/tablas` (la tabla completa de retención o del fondo de
    solidaridad) y `POST|PUT /plataforma/nomina/administradoras`.
  - Configuración: `GET|PUT …/configuracion` (`exonerada_aportes`, art. 114-1 del ET; muéstrelo
    con una advertencia: lo decide el contador).
  - Novedades: `POST …/contratos/{id}/novedades` con `tipo` (`TipoNovedad`); según su medida
    (`MedidaNovedad` en la respuesta) llevan `cantidad` (horas o pesos) en una fecha, o `desde` y
    `hasta` (días). Las dominicales solo en domingo o festivo (422). `GET …/novedades?desde=&hasta=
    &contrato_id=` y `POST …/novedades/{id}/anulacion` con `motivo` (204). Sin
    `personas.datos_sensibles.ver` las incapacidades no aparecen; sin `nomina.salarios.ver` los
    valores llegan en `0`.
  - Periodos (`EstadoPeriodoNomina`): `POST …/periodos` (`periodicidad` y `desde` = día 1, o 16 en
    la quincenal) → `…/liquidacion` (se repite mientras no esté aprobado) → `…/aprobacion` (**otra
    cuenta**: a quien liquidó muéstrele «pendiente de aprobación»; 409 si hubo novedades después
    de liquidar: ofrezca liquidar de nuevo) → `…/pago` con `fecha_pago`. `GET …/periodos/{id}`
    (exige `nomina.salarios.ver`) trae cada liquidación con sus líneas (`ConceptoNomina`,
    `ClaseLineaNomina`): agrúpelas en devengos, deducciones y aportes del empleador. Si falta un
    parámetro o una tabla vigente, liquidar responde 422 nombrándolo (y el código del empleado).
  - Desprendibles (periodo aprobado o pagado; antes, 409): `GET …/liquidaciones/{id}/desprendible`
    (PDF, `nomina.salarios.ver`) y `POST …/periodos/{id}/desprendibles` (los envía por correo;
    devuelve `enviados` y `sin_correo`: ofrezca descargar los de quienes no tienen correo).
  - **Mi nómina** (cualquier cuenta, sin roles en la entidad): `GET /yo/nomina/desprendibles`
    (todas las entidades donde trabaja) y `GET /yo/nomina/{entidad_id}/desprendibles/{id}` (PDF).
    Muestre la sección en la app solo si la lista no viene vacía.
  - Prestaciones: no tienen pantalla propia de pago. La prima llega sola en los periodos de junio
    y diciembre, los intereses en el primer periodo de enero y, al terminar un contrato, el
    periodo de la terminación trae cesantías, intereses, prima, vacaciones compensadas e
    indemnización: **liquide el periodo después de registrar la terminación**. Las líneas de
    clase `provision` son informativas (no las sume al neto). Registre las vacaciones
    disfrutadas como novedad `vacaciones` (días calendario). `GET …/cesantias?anio=` da las
    cesantías por consignar por fondo.
  - Con contabilidad, aprobar y pagar contabilizan solos. El contador debe tener reglas para los
    eventos `nomina.liquidacion` (un concepto por cada valor de `ConceptoNomina` que use la
    entidad, más `neto`; los aportes del empleador y las provisiones con débito y crédito) y
    `nomina.pago` (`neto` y `girado`). Sin regla, aprobar responde 422 nombrando el evento:
    lleve al contador a la configuración de reglas.
  - Reportes (`nomina.salarios.ver`, solo con lo aprobado): `GET …/aportes?anio=&mes=` (por
    empleado y por administradora, con el tipo de cotizante: para diligenciar la PILA en el
    operador) y `GET …/ingresos-retenciones?anio=` (totales por `GrupoCertificado` para el
    certificado de ingresos y retenciones).
  - Centro de costo: `condiciones.centro_costo_id` (solo si `capacidades.modulos` trae
    `contabilidad`; si no, 422). Con contabilidad, `POST …/contabilizacion` con `desde` y `hasta`
    contabiliza la nómina aprobada antes de tenerla (informe con `contabilizados` y `fallidos`).
  - `POST …/contratos/{id}/simulacion-terminacion` (fecha y motivo; no guarda nada) muestra el costo
    del retiro antes de registrarlo; `GET …/contratos/{id}/vacaciones?fecha=` da el saldo
    (causados, disfrutados y pendientes) para la ficha.
  - Novedades por planilla: `GET …/novedades/importaciones/plantilla`, `POST
    …/novedades/importaciones?nombre=` y `…/{id}/confirmar` (empleado por su `codigo`).
  - Deducciones de la retención del empleado (con su soporte en `/archivos`): `GET|POST
    …/empleados/{id}/deducciones` (`TipoDeduccion`; `dependientes` sin valor) y `POST
    …/deducciones/{id}/terminacion` con `hasta`. Se aplican desde la liquidación siguiente.
  - Certificado laboral: `GET …/empleados/{id}/certificado-laboral?con_salario=` (PDF; con salario
    exige `nomina.salarios.ver`) y, para el empleado, `GET /yo/nomina/{entidad_id}/certificado-laboral`
    (con su salario).
  - Avisos automáticos (no tienen pantalla): quienes tienen `nomina.empleados.gestionar` reciben por
    correo los contratos a término fijo que vencen en 45 días (para entregar el preaviso a tiempo) y
    los recordatorios de intereses y consignación de cesantías y de la prima.

## 4. Menú: maqueta 3D, zonas y módulos

El menú **es la maqueta**: una plantilla 3D fija por tipo de entidad (conjunto para la PH,
oficina para la empresa) en la que cada **zona** abre una o más **secciones**, y cada sección
lista sus **acciones**. La maqueta es igual para todos los clientes; por cliente solo cambia qué
se enciende. Todo lo decide una sola llamada al entrar: `GET /entidades/{id}/capacidades`
(`tipo`, `permisos`, `modulos`), más los `roles` que trae `GET /yo/entidades`.

### Cómo se calcula el estado de cada zona

Para cada sección, en este orden:

1. **Próximamente**: aún no existe en el backend (hoy: nómina electrónica, plan 16).
2. **No contratada**: no se cumple su regla de módulos. Zona **apagada** (gris, candado, «Módulo
   no contratado»), nunca oculta: es una vitrina comercial. Al tocarla, muestre qué incluye.
3. **Sin acceso**: el módulo está, pero la cuenta no tiene **ninguno** de los permisos de sus
   acciones (zona tenue, «Sin acceso para su rol»).
4. **Activa**: al menos una acción visible.

La zona toma el mejor estado de sus secciones. Dentro de una sección activa, muestre solo las
acciones cuyo permiso tiene la cuenta (basta uno de la lista). Las reglas de módulos son de tres
clases: **ninguno** (incluido con la entidad), **uno** y **alguno de** (sirve con cualquiera de
los dos). El backend vuelve a exigir todo: si una acción visible responde 403
`modulo_no_habilitado` o `sin_permiso`, vuelva a pedir las capacidades y repinte la maqueta.

### Módulos contratables (`Modulo`)

| Módulo | Qué enciende | Tipo de entidad | Notas |
|---|---|---|---|
| *(incluido)* | Unidades y residentes (bienes privados, propietarios, coeficientes, censo, importación), datos y usuarios (roles, auditoría, archivos, protección de datos, organizaciones), directorio de comercios | PH; en empresa solo roles, auditoría y archivos | No se contrata. |
| `bienes_comunes` | Zonas comunes: reservas, disponibilidad, parqueaderos y sorteos | PH | |
| `control_acceso` | Portería: visitas, correspondencia, minuta, bitácora, credenciales, puentes | PH | |
| `asambleas` | Asambleas y votaciones, papeleta virtual | PH | |
| `gestion_administrativa` | PQRS, comunicados, chat de WhatsApp, mantenimiento e inventario | PH | |
| `contabilidad_ph` | Cartera de la PH: liquidación de expensas, recaudos, mora, paz y salvos, presupuesto | PH | Exige además `contabilidad`. |
| `contabilidad` | Contabilidad, tesorería, declaraciones y exógena; además terceros, perfil tributario y facturas de venta | PH y empresa | |
| `facturacion_electronica` | Documentos electrónicos ante la DIAN, recibidas y RADIAN, habilitación; además terceros, perfil tributario y facturas de venta | PH y empresa | Se vende solo (ADR 0022). |

### Catálogo de la PH (plantilla `conjunto`)

| Zona | Sección | Módulos | Acciones (permiso que la muestra) | Rutas principales |
|---|---|---|---|---|
| Torres | Unidades y residentes | ninguno | Bienes privados (`bienes_privados.ver`) · Propietarios (`titulares.ver`) · Censo (`censo.ver`) · Aprobar cambios del censo (`censo.aprobar`) · Coeficientes (`bienes_privados.ver`; editar con `bienes_privados.coeficientes.editar`) · Importar planilla (`importaciones.ejecutar`) · Libros art. 51 (`censo.exportar`) | `/copropiedades/{id}/{agrupaciones,bienes-privados,residencias,vehiculos,mascotas,cambios-censo,cuadros-coeficientes,importaciones,libros}` |
| Administración | Gestión administrativa | `gestion_administrativa` | PQRS (`tickets.ver_todos`, `tickets.atender`, `tickets.crear`) · Comunicados (`comunicados.gestionar`) · Chat de WhatsApp (`conversaciones.atender`) · Preguntas frecuentes (`preguntas_frecuentes.editar`) · Mantenimiento (`mantenimiento.planificar`, `mantenimiento.ejecutar`, `mantenimiento.verificar`) · Inventario (`inventario.ver`) | `…/{tickets,comunicados,conversaciones,preguntas-frecuentes,ordenes-trabajo,planes-mantenimiento,obligaciones,activos}` |
| Administración | Cartera | `contabilidad_ph` | Saldos y edades (`cartera.ver`) · Liquidación (`cartera.liquidar`) · Recaudos (`tesoreria.recibos.registrar`) · Ajustes y acuerdos (`cartera.ajustar`) · Paz y salvo (`cartera.certificados.emitir`) · Presupuesto (`presupuesto.gestionar`) · Configuración (`cartera.configurar`) | `/copropiedades/{id}/cartera/…` |
| Administración | Contabilidad, Tesorería, Tributario, Facturación | como en la empresa (abajo) | | `/entidades/{id}/…` |
| Administración | Datos y usuarios | ninguno | Ficha (`copropiedad.datos.ver`) · Usuarios y roles (`roles.ver`) · Auditoría (`auditoria.ver`) · Protección de datos (`proteccion_datos.gestionar`) · Organizaciones y contratos (`organizaciones.vincular`, `contratos.gestionar`) · Archivo (`archivos.ver`) | `/copropiedades/{id}`, `/entidades/{id}/{roles,asignaciones,auditoria,archivos}`, `…/proteccion-datos`, `…/contratos-servicio` |
| Salón social | Asambleas | `asambleas` | Reuniones y actas (`asambleas.ver`) · Convocar (`asambleas.convocar`) · Conducir y tablero en vivo (`asambleas.conducir`, `asambleas.ver`) · Mesa de registro (`asambleas.registrar`) · Voto presencial (`asambleas.votos.registrar`) | `…/asambleas/…` |
| Salón social, piscina, cancha, gimnasio | Zonas comunes | `bienes_comunes` | Reservar (`reservas.crear`) · Disponibilidad (`bienes_comunes.ver`) · Aprobar reservas (`reservas.aprobar`) · Entregas y recibos (`reservas.entregas.registrar`) · Recursos y reglas (`bienes_comunes.ver`; editar con `bienes_comunes.editar`) · Restricciones (`reservas.restringir`) | `…/{recursos,reservas,restricciones-reserva}` |
| Parqueaderos | Zonas comunes | `bienes_comunes` | Parqueaderos (`parqueaderos.ver`; asignar con `parqueaderos.asignar`) · Sorteos (`sorteos.gestionar`; ver con `bienes_comunes.ver`) | `…/{parqueaderos,sorteos}` |
| Portería | Portería | `control_acceso` | Visitas (`acceso.registrar`, `acceso.visitantes.autorizar`) · Correspondencia (`acceso.correspondencia.entregar`, `acceso.registrar`) · Bitácora (`acceso.bitacora.ver`) · Minuta (`acceso.minuta.ver`, `acceso.minuta.registrar`) · Credenciales y autorizaciones (`acceso.credenciales.gestionar`) · Biometría (`acceso.biometria.enrolar`) · Puntos, puentes y configuración (`acceso.configurar`) | `…/{visitas,correspondencia,eventos-acceso,minuta,credenciales,autorizaciones-acceso,puntos-acceso,puentes,acceso}` |
| Pantalla del lobby | Directorio | ninguno (la publicidad no es un módulo) | Directorio (`directorio.ver`) · Reseñar (`directorio.resenar`) · Enlace para los grupos de WhatsApp (`directorio.compartir`) | `/entidades/{id}/directorio/…`, `/entidades/{id}/publicidad/{piezas,autorizacion}`, reseñas y enlace en `/copropiedades/{id}/directorio/…` |

**Zonas comunes concretas.** La maqueta trae salón, piscina, cancha y gimnasio genéricos; cada
PH crea sus recursos con el nombre que quiere, pero al crearlos elige su **categoría**
(`CategoriaBienComun`). Esa categoría es el vínculo con la zona:

| Zona de la maqueta | Consulta |
|---|---|
| Salón social | `GET …/recursos?categoria=salon_social` (y `salon_comunal`) |
| Piscina | `GET …/recursos?categoria=piscina` |
| Cancha | `GET …/recursos?categoria=cancha` |
| Gimnasio | `GET …/recursos?categoria=gimnasio` |
| Parqueaderos | `GET …/parqueaderos` (o `…/recursos?tipo=parqueadero`) |

Con un solo recurso, abra ese directamente; con varios, la lista; con ninguno, muestre la zona
tenue con «Esta copropiedad no tiene …». Las demás categorías (BBQ, sauna, coworking, terraza…)
no tienen pieza en la maqueta: muéstrelas en la lista de Zonas comunes.

**La pantalla del lobby** no se configura desde la PH: su token lo genera la plataforma
(`POST /plataforma/publicidad/pantallas/{copropiedad_id}`) y la pantalla consulta
`…/publicidad/pantalla` con `X-Pantalla`. En la maqueta esa zona abre el directorio.

### Catálogo de la empresa (plantilla `oficina`)

| Zona | Sección | Módulos | Acciones (permiso que la muestra) | Rutas principales |
|---|---|---|---|---|
| Recepción | Datos y usuarios | ninguno | Ficha (`copropiedad.datos.ver`; editar con `copropiedad.datos.editar`) · Usuarios y roles (`roles.ver`) · Auditoría (`auditoria.ver`) | `/entidades/{id}`, `/entidades/{id}/{roles,asignaciones,cuentas,auditoria}` |
| Contabilidad | Contabilidad | `contabilidad` | Comprobantes (`contabilidad.ver`; crear con `contabilidad.comprobantes.crear`) · Plan de cuentas y centros de costo (`contabilidad.ver`) · Periodos y cierre (`contabilidad.ver`; cerrar con `contabilidad.periodos.cerrar`) · Reportes y estados financieros (`contabilidad.ver`) · Configuración y plantillas (`contabilidad.configurar`) | `/entidades/{id}/contabilidad/…` |
| Tesorería | Tesorería | `contabilidad` | Recibos de caja (`tesoreria.recibos.registrar`) · Egresos (`tesoreria.egresos.registrar`, `tesoreria.egresos.autorizar`) · Cuentas por pagar (`tesoreria.facturas_proveedor.gestionar`) · Conciliación (`tesoreria.conciliar`) · Cartera de clientes (`contabilidad.ver`) | `…/contabilidad/{tesoreria,cuentas-por-pagar,cuentas-por-cobrar}` |
| Facturación | Facturación | alguno de `facturacion_electronica`, `contabilidad` | Facturas de venta (`facturacion.ver`; crear con `facturacion.facturas.gestionar`) · Catálogo de productos (`facturacion.items.gestionar`) · Compras y documento soporte (`facturacion.facturas.gestionar`) · Libros y paquete para el contador (`facturacion.exportar`) · **Solo con `facturacion_electronica`:** emitir ante la DIAN, notas, transcripción de contingencia y aceptación tácita (`facturacion.emitir`), recibidas y eventos RADIAN (`facturacion.recibir`), habilitación DIAN (`facturacion.configurar`) | `/entidades/{id}/facturacion/…` |
| Tributario | Tributario | alguno de `contabilidad`, `facturacion_electronica` | Terceros (`terceros.gestionar`; ver con `contabilidad.ver` o `facturacion.ver`) · Perfil tributario (`tributario.configurar`, `tributario.perfil.confirmar`) · Resoluciones de numeración (`tributario.configurar`) · **Solo con `contabilidad`:** retención, IVA, ICA y exógena (`tributario.preparar`) | `/entidades/{id}/{terceros,tributario}`, `…/contabilidad/tributario` |
| Archivo | Archivo | ninguno | Documentos (`archivos.ver`) | `/entidades/{id}/archivos` |
| Sala de juntas | Reportes | `contabilidad` | Estados financieros (`contabilidad.ver`) | `…/contabilidad/reportes` |
| Nómina | Nómina | `nomina` | Empleados y contratos (`nomina.ver`; crear, modificar, prorrogar y terminar con `nomina.empleados.gestionar`; salarios y cuentas con `nomina.salarios.ver`) · Importar empleados (`importaciones.ejecutar` y `nomina.empleados.gestionar`) · Catálogo laboral vigente (`nomina.ver`) · Novedades (`nomina.novedades.registrar`) · Periodos y liquidación (`nomina.liquidar`; aprobar con `nomina.aprobar`, ver con `nomina.salarios.ver`) · Configuración (`nomina.configurar`) · Desprendibles (`nomina.salarios.ver`; enviar con `nomina.liquidar`) · Cesantías por consignar (`nomina.salarios.ver`) · Aportes del mes e ingresos del año (`nomina.salarios.ver`) · **Próximamente:** archivo plano de la PILA y nómina electrónica | `/entidades/{id}/nomina/…` |

En una PH, estas mismas secciones se abren desde la zona Administración, con las rutas de
`/entidades/{id}` (mismo id que la copropiedad). La **ficha de la empresa** (nombre, razón
social, NIT, dirección, contacto, zona horaria) es `GET/PATCH /entidades/{id}` con
`copropiedad.datos.ver` / `copropiedad.datos.editar`; en la PH esos mismos datos se editan en su
ficha completa (`/copropiedades/{id}`).

### La plataforma (`tipo: plataforma`)

La entidad de la plataforma es una empresa (GISLA): use la plantilla `oficina` para su propia
contabilidad y facturación, y agregue una zona **Centro de operación** que solo se enciende si
`GET /yo` trae `permisos_plataforma` no vacío. Si la sesión no tiene segundo factor, la zona
aparece con candado y lleva a activarlo (las rutas responden 403 `segundo_factor_requerido`).

| Sección del centro de operación | Acciones (permiso de plataforma) | Rutas |
|---|---|---|
| Clientes y módulos | Copropiedades (`plataforma.copropiedades.ver`; alta con `plataforma.copropiedades.crear`) · Empresas (`plataforma.entidades.crear`) · Módulos contratados (`plataforma.modulos.gestionar`) | `/plataforma/{copropiedades,entidades}` |
| Usuarios de la plataforma | Roles y asignaciones (`plataforma.roles.gestionar`, `plataforma.roles.asignar`) · Bloquear cuentas (`plataforma.cuentas.bloquear`) | `/plataforma/{roles,asignaciones,cuentas}` |
| Organizaciones | Reclamaciones (`plataforma.organizaciones.revisar`) | `/plataforma/reclamaciones` |
| Normativa | PUC, retenciones y UVT, IVA, ICA, exógena, tasas de referencia (`plataforma.normativa.gestionar`) · Plantillas contables (`plataforma.plantillas_contables.publicar`) | `/plataforma/{puc,retenciones,uvt,tributario,tasas-referencia,plantillas-contables}` |
| WhatsApp | Líneas y tarifas (`plataforma.whatsapp.gestionar`) | `/plataforma/whatsapp/…` |
| Directorio y publicidad | Anunciantes y categorías (`plataforma.directorio.gestionar`) · Moderación (`plataforma.publicidad.moderar`) · Tarifas y liquidaciones (`plataforma.publicidad.liquidar`) · Pantallas de lobby (`plataforma.modulos.gestionar`) | `/plataforma/publicidad/…` |
| Consumo | Documentos electrónicos por entidad (`plataforma.facturacion.ver`; `desde` y `hasta` incluidos) | `/plataforma/facturacion/consumo` |
| Cobros | Catálogo, tarifas y suscripciones (`plataforma.comercial.gestionar`) · Cortes: calcular, aprobar, descartar (`plataforma.cobros.aprobar`) · Facturar, factura externa y pago (`plataforma.cobros.facturar`) | `/plataforma/cobros/…` |

### Acciones personales (propietarios y residentes)

Propietarios y residentes casi no tienen permisos de administración: su plantilla trae
`reservas.crear`, `tickets.crear`, `acceso.visitantes.autorizar`, `bienes_comunes.ver`,
`directorio.ver` y, el propietario, `asambleas.ver`. Con las reglas de arriba ya ven encendidas
zonas comunes (reservar), portería (sus visitas y paquetes), administración (radicar PQRS),
salón social (asambleas) y la pantalla del lobby (directorio). Lo demás de su vida en la PH no
depende de un permiso sino de su **vínculo** con un bien. Muéstrelo cuando los `roles` de
`/yo/entidades` incluyan `propietario` o `residente` (son derivados: los da el backend):

| Acción | Módulos | Ruta | Dónde ponerla |
|---|---|---|---|
| Mis bienes | ninguno | `…/mis-bienes-privados` | Torres (resalte su torre) |
| Mi cartera, cuentas de cobro y paz y salvo | `contabilidad_ph` | `…/mi-cartera/…` | Administración |
| Mis comunicados | `gestion_administrativa` | `…/mis-comunicados` | Administración y aviso en el HUD |
| Mis PQRS | `gestion_administrativa` | `GET /yo/tickets` (todas sus PH) | Administración |
| Mis reservas | `bienes_comunes` | `…/reservas?mias=true` | Zonas comunes |
| Proponer cambios del censo | ninguno | `POST …/cambios-censo`, `…?mias=true` | Torres |
| Mis datos y autorizaciones | ninguno | `…/proteccion-datos/mis-datos` | Panel de cuenta |
| Publicidad: autorizar o retirar | ninguno | `…/publicidad/autorizacion` | Panel de cuenta |

La **papeleta** de una asamblea no va en el menú: llega como enlace personal (cabecera
`X-Papeleta`, §13). Para todos, un botón fijo del HUD lleva a **Mi cuenta** (`/yo`: perfil,
contraseña, segundo factor, sesiones y dispositivos) y al selector de entidad.

### Ajustes al mapa actual del frontend

Comparado con `src/escena/mapa-modulos.ts` y `src/escena/plantillas/zonas.ts` del repositorio
del frontend (los permisos que usa existen todos en el catálogo):

1. **Regla de módulos «alguno de»**: hoy cada sección tiene un solo `modulo`. Tributario (perfil,
   resoluciones, terceros) y Facturación (ventas, catálogo, compras) sirven con
   `facturacion_electronica` **o** `contabilidad`; marcarlos solo con uno apaga la zona a quien
   contrató el otro. Además, algunas acciones exigen un módulo propio dentro de una sección ya
   activa: emitir ante la DIAN, recibidas y habilitación (`facturacion_electronica`), y
   declaraciones y exógena (`contabilidad`). Conviene un campo `modulos` en la sección (alguno
   de) y uno opcional en la acción.
2. **Publicidad y directorio**: la publicidad ya no es un módulo (ADR 0023): el directorio y las
   piezas salen en toda PH y empresa que la plataforma no haya excluido. La acción «Pantallas» con `copropiedad.datos.ver` no
   tiene ruta en la PH: las pantallas las gestiona la plataforma. Cambie la acción por «Enlace
   para los grupos» (`directorio.compartir`).
3. **Ficha y protección de datos**: la ficha sirve a toda entidad con `GET/PATCH /entidades/{id}`
   (en la PH, la completa está en `/copropiedades/{id}`); la protección de datos existe solo para
   copropiedades (`soloPH` en esa acción).
4. **Tipo `plataforma`**: `plantillaDe` lo trata como empresa. Agregue la zona del centro de
   operación descrita arriba.
5. **Acciones personales**: agregue las de la tabla anterior, condicionadas por los roles
   `propietario` y `residente` en lugar de por permisos.
6. **Acciones que faltan**: aprobar cambios del censo, importación y libros (Torres); preguntas
   frecuentes (Administración); ajustes, acuerdos y configuración de la cartera; mesa de registro
   y voto presencial (Asambleas); entregas y restricciones (Zonas comunes); credenciales,
   biometría (Portería); catálogo, compras y documento soporte, libros y habilitación DIAN
   (Facturación); terceros y resoluciones (Tributario); configuración contable (Contabilidad);
   organizaciones y contratos (Datos y usuarios).

## 5. Perfiles y módulos de la propiedad horizontal

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
| Soporte de la plataforma | `capacidades.modo_soporte = true` | La entidad en solo lectura con un aviso fijo; oculte toda acción que escriba (responde 403). |

**Rutas del propio usuario** (no exigen permisos de administración, solo el vínculo):
`/yo/…` (perfil, correo, contraseña, segundo factor, sesiones, dispositivos, `GET /yo/tickets`
de todas sus PH), y en cada PH `…/mis-bienes-privados` (donde es titular), `…/mis-residencias`
(donde reside, también si no es titular: con su calidad y si puede reservar), `…/mi-cartera/…`,
`…/mis-comunicados`,
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
5. Usuarios y roles: crear cada usuario con su rol (`POST /entidades/{id}/usuarios`) o en lote
   (`…/usuarios/importaciones`, ver §7 «Usuarios»); ver el equipo en `GET …/usuarios` y los roles
   en `/entidades/{id}/roles`. A una cuenta ya existente se le asigna otro rol con
   `…/asignaciones`. Nadie otorga lo que no tiene: un 403 al asignar significa eso.
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
`POST …/reservas` responde `aprobada` (confirmación inmediata) o `solicitada`; el bien a cuyo
nombre se reserva se elige de `…/mis-bienes-privados` o `…/mis-residencias` (un tenedor o un
familiar no es titular), y la reserva trae `recurso_nombre` y `bien_codigo` para mostrarla. Ciclo:
solicitada → aprobada → en_curso (`…/entrega`) → finalizada o con_novedad (`…/recibo`), o
cancelada, rechazada, no_asistio. Cancelar tarde puede generar penalidad. Un bien en mora o
sancionado no reserva (`…/restricciones-reserva`). **Cobro**: el contador elige por recurso
(`PUT …/recursos/{id}/cobro`) `manual` (se registra en `…/pago`), `cartera` (al aprobarse queda
como cargo y sale en la cuenta de cobro) o `factura` (con un ítem; `POST …/reservas/{id}/factura`
crea la factura en borrador, una por reserva). Con `cartera` o `factura`, `…/pago` responde 422. Parqueaderos y sorteos con semilla publicada
(muestre el compromiso al crear y la semilla al ejecutar: es la prueba de transparencia).

### Tickets y chat (`tickets`, `conversaciones`, módulo `gestion_administrativa`)
Número `PQR-AAAA-NNNNN`, vencimiento calculado en días u horas hábiles (muestre el semáforo con
`vence_en` y `vencido`). Estados en `EstadoTicket`; las transiciones permitidas dependen del papel de la cuenta
(gestiona, atiende o es el solicitante): el detalle (`GET …/tickets/{id}`) trae
`transiciones_permitidas` calculadas para quien consulta; muestre solo esos botones (cada uno se
acepta en `POST …/transiciones`).
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
verificada (**quien ejecuta no verifica**: oculte el botón cuando `ejecutada_por` es la cuenta
que consulta; si lo intenta, 409 `conflicto`). Obligaciones
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
   (§14), puntos que avanzan, salidas y reingresos. Sin quórum en primera:
   `…/segunda-convocatoria`.
5. **Votación**: abrir la pregunta (habilita a los presentes y les llega por WhatsApp) → votos por
   web (papeleta), WhatsApp, presencial o rótulo → cerrar (guarda el escrutinio). El asistente
   vota **una vez por todos sus bienes**; un segundo intento da 409 `ya_voto` (y una pregunta ya cerrada,
   409 `conflicto_estado`); si el asistente no está presente o no representa el bien, 403
   `no_inscrito`. El escrutinio muestra coeficiente **y** unidades. En preguntas
   **reservadas** los resultados públicos son totales; el detalle exige
   `asambleas.votos.ver_detalle` y cada consulta queda auditada (adviértalo en la interfaz).
   El escrutinio (`Escrutinio` en `…/resultados`, en el tablero y en el WebSocket) trae
   `habilitados`, `votantes`, `sin_votar` y `total` (`ConteoEscrutinio`: coeficiente y
   unidades), las `opciones` (`ResultadoOpcion`) y la `decision`, etiquetada por su campo
   `decision` (`aprobada`, `no_aprobada`, `ganadora`, `sin_ganadora`, `por_opcion` o `reparto`,
   que lleva el cociente y los `cupos` por plancha). `regla` tiene la misma forma que
   `regla_mayoria` de la pregunta (`calificada_70`).
6. **Cierre y acta**: `GET …/acta` (revisar, esquema `DatosActa`) → `POST …/acta` (PDF) → `PUT …/acta/estado`
   (escaneo firmado, aprobada, publicada). Elecciones de consejo o revisor: `…/eleccion` crea los
   roles. Reportes de asistencia, votos, canales y costo de WhatsApp.

**Papeleta virtual** (sin sesión, cabecera `X-Papeleta`, §13): `GET …/papeleta`, ingreso, salida y
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
- **Elegir el bien o la persona desde la portería**: `GET …/acceso/bienes?q=` (con
  `acceso.registrar`; solo `id` y `codigo` de los bienes activos) para anunciar y recibir, y
  `GET …/acceso/personas?q=` (con `acceso.credenciales.gestionar` o `acceso.biometria.enrolar`;
  `nombre` y `documento` enmascarado `****3456`, mínimo 2 caracteres) para credenciales y
  biometría. Las plantillas `vigilante` y `supervisor_seguridad` no traen `bienes_privados.ver`
  ni `personas.ver`: use estas rutas en la portería, no `…/bienes-privados` ni `…/personas`.

### Protección de datos (`proteccion_datos`)
Política versionada; la persona autoriza por **finalidad** bajo la política vigente y puede
revocar. Muestre la política antes de pedir la autorización. Solicitudes del titular (consulta,
reclamo…) con vencimiento en días hábiles y una prórroga única: muestre «quedan N días hábiles»
con `dias_habiles_restantes` (0 = vence hoy o venció; `null` si ya se respondió). Cada solicitud
trae `persona_nombre` del titular (no hace falta `personas.ver`).
«Mis datos» (`…/proteccion-datos/mis-datos`) para el titular.

### Directorio y publicidad (`directorio`, `publicidad`)
El directorio y las piezas funcionan en toda **PH y empresa** (`/entidades/{id}/directorio/…`,
`/entidades/{id}/publicidad/…`); las destacadas y las piezas salen salvo que la plataforma haya
excluido la entidad (ADR 0023). Solo en la PH: reseñas anónimas (una por cuenta, solo
propietarios y residentes vigentes) y su reporte, «busco …» por WhatsApp, el enlace público, la
pantalla del lobby y los masivos. «Busco …»: `POST …/directorio/busquedas` y **solo** con el «Sí» explícito
(`…/contacto`) se comparte el contacto con esos proveedores: haga ese consentimiento visible y
específico. Piezas publicitarias solo para quien autorizó la publicidad de la plataforma
(`…/publicidad/autorizacion`; la entidad no puede autorizar por el titular). Pantalla del lobby
con `X-Pantalla`. El enlace del directorio para los grupos es **estable**: `GET
…/directorio/enlace-publico` devuelve el mismo `token` mientras no se regenere ni se revoque.

### Organizaciones y contratos (`organizaciones`)
Administradoras, contables, vigilancia… son organizaciones globales que se buscan por **NIT
exacto** antes de crearlas (para no duplicarlas). Un contrato de servicio vincula la organización
con la PH y su personal recibe roles **derivados** que vencen con el contrato. Cada contrato
trae su `organizacion` (id, razón social y NIT).

### Consola de la plataforma (`plataforma`)
Exige segundo factor (si falta: 403 `segundo_factor_requerido`, lleve a `/yo/segundo-factor`).
Alta de copropiedades y entidades (con su administrador, nuevo o existente), estado y **módulos**
de cada una, usuarios y roles de plataforma (`POST /plataforma/cuentas`), consulta, bloqueo y
desbloqueo de cuentas (`GET /plataforma/cuentas?identificador=`, `…/bloquear`, `…/desbloquear`),
reclamaciones de organizaciones, líneas y tarifas de WhatsApp, catálogos normativos
(PUC, retenciones y UVT, IVA, ICA, exógena, tasas de referencia, catálogo laboral), plantillas
contables, consumo de facturación electrónica, moderación del directorio y la publicidad.

- **Entidades** (`/plataforma/entidades`): PH, empresas y la plataforma en una tabla (tipo, estado,
  módulos; filtros `q` por nombre o NIT, `tipo` y `estado`, aplicados en todas las páginas);
  `GET …/{id}` da una sola (la ficha del cliente) y `POST …/{id}/estado` cambia el de empresas y
  PH (la ruta de copropiedades queda como alias).
- **Roles de plataforma de una cuenta**: `GET /plataforma/asignaciones?cuenta_id=` da el historial
  (vigentes con `vigente_hasta` nulo) con el `id` que se pasa a `DELETE /plataforma/asignaciones/{id}`.
- **Usuarios de cualquier entidad** (`plataforma.entidades.usuarios`): `/plataforma/entidades/{id}/
  {roles,asignaciones,usuarios}` con los mismos esquemas que `/entidades/{id}/…`.
- **Soporte** (`plataforma.soporte.ver`): `POST /plataforma/entidades/{id}/soporte` con `motivo`
  abre 2 h de solo lectura; luego se navega la entidad con las rutas normales
  (`capacidades.modo_soporte`). Nunca muestra datos sensibles, votos reservados ni convivencia.
- **Publicidad** (`plataforma.publicidad.gestionar`, ADR 0023): alta del comercio y su ficha,
  campañas y piezas en `/plataforma/publicidad/anunciantes/{id}/campanas/…`, métricas, leads, el
  enlace de cupones del comercio (`…/enlace-cupones`: `GET` lo vuelve a entregar, `POST` lo
  reemplaza, `DELETE` lo revoca) y exclusiones de entidades (`/plataforma/publicidad/exclusiones`).
  **Otra cuenta** modera (quien cargó o editó recibe 409 `conflicto`). El token de la pantalla del
  lobby también se vuelve a consultar con `GET /plataforma/publicidad/pantallas/{id}`.

- **Cobros** (ADR 0024, `/plataforma/cobros/…`): catálogo de productos con tarifas versionadas
  (`ModeloTarifa`; una tarifa nueva desde una fecha, nunca se edita), suscripción de cada entidad
  (una vigente; ciclo en meses o por evento, prueba, plazo y pagador) con sus líneas, y la bandeja
  de **cortes** (`?estado=borrador` para revisar). Un corte igual al anterior llega `aprobado` con
  `aprobado_por: null`. `…/facturar` emite la factura en la entidad de la plataforma; mientras no
  facture en el sistema use `…/factura-externa`. `…/pago` registra el pago y, si la entidad estaba
  suspendida por mora, la reactiva. Las liquidaciones de publicidad se cobran con
  `POST …/liquidaciones/{id}/corte`. Al habilitar un módulo (`POST
  /plataforma/copropiedades/{id}/modulos`) la respuesta trae `linea_suscripcion_id` si se agregó
  su línea de cobro; si viene `null`, el módulo sigue en `modulos_sin_cobro`. La liquidación de
  publicidad pasa a `facturada` y `pagada` con su corte. La suspensión (manual o por mora)
  responde 409 con una
  asamblea instalada.

### Mi suscripción (la entidad)
`GET /entidades/{id}/suscripcion` (`suscripcion.ver`, administrador): plan contratado, consumo del
periodo en curso y facturas (cortes aprobados, facturados y pagados). 404 si no tiene suscripción.

### Entidad suspendida
Si `capacidades.entidad_suspendida` es `true`, `permisos` ya viene recortado: solo lecturas, la
portería (`acceso.*`) y la atención a titulares (`proteccion_datos.*`). Muestre un aviso
permanente («La cuenta está suspendida; comuníquese con la plataforma») y oculte las acciones que
no estén en `permisos`; si aun así llega un 403 `sin_permiso`, es por la suspensión.

## 6. Estados de los recursos

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
| Entidad / copropiedad | `EstadoCopropiedad`: en_configuracion, activa, suspendida, retirada | Avise si la entidad no está activa; suspendida = solo lectura. |
| Corte de cobro | `EstadoCorte`: borrador → aprobado → facturado → pagado; o descartado | Descartar solo antes de facturar. |
| Suscripción | `EstadoSuscripcion`: activa, suspendida, terminada | `suspendida_por_mora` dice si el pago la reactiva. |
| Empleado | `EstadoEmpleado`: activo, retirado | Derivado de sus contratos. |
| Contrato laboral | Sin `estado`: `fecha_terminacion` nula = vigente | Terminado no acepta versiones ni prórrogas. |
| Periodo de nómina | `EstadoPeriodoNomina`: abierto → liquidado → aprobado → pagado | Aprueba otra cuenta; aprobado no cambia. |

## 7. Sesión

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

### Usuarios (sin registro abierto, plan 20)

- **Quién crea a quién.** El superusuario y quien tenga `plataforma.cuentas.crear` crean usuarios
  de la plataforma (`POST /plataforma/cuentas`, con `rol_plataforma_id` opcional). Al dar de alta
  una PH o empresa (`POST /plataforma/copropiedades` o `/plataforma/entidades`) se envía
  `administrador: { nombre, correo, usuario? }` (o `administrador_cuenta_id`, uno de los dos). El
  administrador de cada entidad crea su equipo con `POST /entidades/{id}/usuarios`
  (`nombre`, `correo`, `rol_id`, opcionales `usuario`, `telefono`, `documento`; exige
  `roles.asignar` y nunca un rol con permisos que él no tiene).
- **Una persona, una cuenta.** Si el correo ya tiene cuenta (trabaja en otra PH o empresa), no se
  crea otra: se le asigna el rol y le llega un aviso; la respuesta trae `cuenta_nueva: false`.
  Solo es 409 si el `usuario` pedido es de otra cuenta o la cuenta de ese correo está bloqueada.
  Se crea o vincula también su persona en la entidad (`persona_id`).
- **Definir la contraseña.** Le llega por correo un código de 6 dígitos (vence en 72 h). La
  pantalla «Definir contraseña» envía `correo`, `codigo` y `contrasena_nueva` a
  `POST /auth/contrasena/restablecer` (la misma de «olvidé mi contraseña»). Mientras tanto no
  puede iniciar sesión. Reenviar: `POST /entidades/{id}/usuarios/{cuenta_id}/codigo-contrasena`;
  `GET /entidades/{id}/usuarios` muestra `contrasena_pendiente` y sus roles.
- **Carga masiva** por entidad: `GET …/usuarios/importaciones/plantilla` (CSV con `nombre`,
  `correo`, `rol` (código) y opcionales `usuario`, `telefono`, `documento`) →
  `POST …/usuarios/importaciones?nombre=archivo.csv` (cuerpo crudo; vista previa con errores por
  fila) → `POST …/usuarios/importaciones/{id}/confirmar` (todo o nada; un choque dice la fila).
- **Inactivar.** En una entidad: terminar sus asignaciones (`DELETE …/asignaciones/{id}`). En
  toda la plataforma: bloquear la cuenta (cierra sus sesiones) y desbloquearla con un motivo.

## 8. Errores

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
| `no_autenticado` | 401 | Falta la sesión o venció. | Renovar una vez (§7); si falla, ir al inicio de sesión. |
| `credenciales_invalidas` | 401 | Usuario, contraseña o código incorrectos. | Mensaje en el formulario, sin decir cuál de los datos falló. |
| `sin_permiso` | 403 | La cuenta no tiene el permiso. | Volver a pedir `capacidades` y ocultar la acción. |
| `segundo_factor_requerido` | 403 | La plataforma exige segundo factor. | Llevar a activar el TOTP (`/yo/segundo-factor`). |
| `no_inscrito` | 403 | El asistente no está presente o no representa el bien. | En la papeleta: invitar a registrar el ingreso o acudir a la mesa. |
| `enlace_invalido` | 403 | Enlace firmado vencido o alterado. | Pedir un enlace nuevo (§12) y abrirlo de una vez. |
| `modulo_no_habilitado` | 403 | La entidad no contrató el módulo. | Ocultar el menú; si llega, pantalla «Módulo no contratado» con contacto comercial. |
| `no_encontrado` | 404 | No existe o la cuenta no tiene roles allí. | Pantalla de «no encontrado»; no insinuar que existe. |
| `conflicto` | 409 | Ya existe o rompe una regla (último administrador, misma cuenta que aprueba…). | Mostrar `mensaje` junto a la acción. |
| `conflicto_estado` | 409 | La acción no aplica en el estado actual. | Recargar el recurso y repintar los botones. |
| `ya_voto` | 409 | Los bienes ya votaron esa pregunta. | Mostrar «Su voto ya quedó registrado» (no es un fallo). |
| `validacion` | 422 | Datos bien formados que rompen una regla de negocio. | Mostrar `mensaje` cerca del formulario. |
| `idempotencia_reusada` | 422 | La `Idempotency-Key` se usó con otro cuerpo. | Error del cliente: generar una clave nueva por acción (§9). |
| `coeficientes_no_suman_uno` | 422 | El cuadro no suma exactamente 1. | Mostrar la diferencia del mensaje junto al total. |
| `regla_contable` | 422 | Partida doble, periodo no abierto, cuenta que no admite la línea… | Mostrar el motivo en el comprobante. |
| `demasiados_intentos` | 429 | Muchos intentos fallidos de inicio de sesión o código. | Bloquear el formulario unos minutos. |
| `demasiadas_peticiones` | 429 | Límite por IP de la ruta. | Esperar `Retry-After` (§11). |
| `interno` | 500 | Error inesperado. | Mensaje genérico con el `X-Request-Id` para soporte. |
| `no_disponible` | 503 | Una dependencia no responde. | Reintentar más tarde con espera creciente. |

## 9. Reintentos (`Idempotency-Key`)

Los `POST` con sesión aceptan `Idempotency-Key` (1 a 200 caracteres). Genere un UUID por
**acción del usuario** (no por intento) y reutilícelo si la red falla y reintenta: durante 24 h
la API devuelve la respuesta original con `Idempotency-Replayed: true`, sin duplicar nada. La
misma clave con otro cuerpo responde 422 `idempotencia_reusada`. En el voto presencial la
cabecera es obligatoria.

## 10. Listados paginados

`?limite=50&despues=<cursor>` (`limite` de 1 a 200, 50 por defecto). La respuesta
(`PaginaRespuesta_…`) trae `datos` y `siguiente`: pase `siguiente` como `despues` para la página
siguiente; `null` indica que es la última. No hay números de página ni totales (por cursor).

## 11. Límites de peticiones

Las rutas sin sesión (inicio de sesión, recuperación, papeleta, enlaces públicos) tienen límite
por IP. Al superarlo: 429 `demasiadas_peticiones` con `Retry-After` (segundos). Espere ese tiempo
antes de reintentar y muestre un aviso; no reintente en bucle.

## 12. Archivos

- **Subir**: `POST /api/v1/entidades/{id}/archivos?nombre=…&clasificacion=…` con el contenido
  crudo en el cuerpo (`Content-Type: application/octet-stream`, no `multipart`). El tipo real se
  valida por contenido; un tipo no admitido o muy grande da 422 `validacion`. La respuesta trae
  el `id` del archivo, que luego se adjunta en otras peticiones (tickets, reservas, actas…).
- **Listar**: `GET /api/v1/entidades/{id}/archivos` (§10, más recientes primero) con
  `clasificacion`, `q` (fragmento del nombre, sin tildes ni mayúsculas), `desde` y `hasta`. Sin
  `archivos.personales.ver` no aparecen los personales ni los sensibles (y filtrar por ellos da
  403).
- **Descargar**: pida `GET …/archivos/{archivo_id}/enlace` y abra la URL que devuelve: es un enlace
  firmado de **5 minutos** que no necesita sesión. Pídalo justo antes de abrirlo; no lo guarde.

## 13. Credenciales sin sesión de persona

| Cabecera | Para qué |
|---|---|
| `X-Papeleta` | Papeleta virtual de una asamblea (el token del enlace personal del asistente). |
| `X-Enlace-Directorio` | Directorio público de la copropiedad (enlace que se comparte en grupos). |
| `X-Pantalla` | Pantalla de publicidad del lobby. |
| `X-Comercio` | El comercio redime cupones (`POST /publicidad/cupones/redimir`); enlace sugerido `https://<web>/comercio#t=<token>`. |

El token llega en el enlace que se le entrega a la persona; el frontend lo lee de la URL y lo
envía en la cabecera, nunca en la ruta de la API.

**Formato acordado de los enlaces** (el backend no arma URLs de la web ni las envía hoy: entrega
los tokens y el frontend construye el enlace). El token va en el fragmento (`#t=…`), que no llega
al servidor ni queda en los registros de Nginx ni en `Referer`:

| Credencial | Enlace de la web | Cabecera hacia la API |
|---|---|---|
| Papeleta | `https://<web>/papeleta/<copropiedad_id>#t=<token>` | `X-Papeleta` |
| Directorio público | `https://<web>/directorio/<copropiedad_id>#t=<token>` | `X-Enlace-Directorio` |
| Pantalla del lobby | `https://<web>/pantalla/<copropiedad_id>#t=<token>` | `X-Pantalla` |

Si algún día el backend envía estos enlaces (p. ej. la papeleta por WhatsApp), usará este mismo
formato con la URL pública de la web en su configuración.

## 14. Tablero en vivo de las asambleas (WebSocket)

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

## 15. Notificaciones push (apps)

Registre el token de Firebase con `POST /api/v1/yo/dispositivos-push` al iniciar sesión y cuando
Firebase lo renueve; elimínelo con `DELETE /api/v1/yo/dispositivos-push/{dispositivo_id}` al
cerrar sesión. Un token inválido se desactiva solo y el aviso se envía por WhatsApp.

## 16. Sugerencias de diseño para aprovechar el backend

- **Permisos y módulos primero.** Cargue `capacidades` al entrar a la entidad y úselas para menús,
  botones y rutas del enrutador. Un 403 no debería verse en el uso normal.
- **Botones según el estado** (§6): el backend rechaza lo que no aplica; la interfaz no debería
  ofrecerlo. Tras cada acción use la respuesta (trae el recurso actualizado) en vez de volver a
  pedir la lista.
- **Doble clic y redes móviles**: genere una `Idempotency-Key` por acción en todo `POST` (pagos,
  recaudos, votos, emisiones, radicaciones) y deshabilite el botón mientras espera.
- **Descargas** (PDF, CSV, ZIP): las rutas con sesión exigen `Authorization`, así que no sirven
  como `<a href>`. Pídalas con `fetch`, cree un `Blob` y use el nombre de `Content-Disposition`.
  Los archivos del almacén van por enlace firmado (§12). Si viene `X-Contenido-Sha256`, muéstrelo
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
- **Listas largas**: paginación por cursor (§10), ideal para desplazamiento infinito; no hay total,
  así que no muestre «página X de Y».
- **Tiempo real**: el tablero de la asamblea va por WebSocket (§14). Para lo demás (visitas en
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

## 17. Cuando cambia el contrato

Cada PR del backend que cambia la API trae el `openapi.json` regenerado y el índice
[guia-frontend-endpoints.md](guia-frontend-endpoints.md). Regenere el cliente, compile y revise
los errores de tipos: un cambio incompatible (campo eliminado o renombrado, valor de enum
retirado) aparece ahí antes de llegar a producción.
