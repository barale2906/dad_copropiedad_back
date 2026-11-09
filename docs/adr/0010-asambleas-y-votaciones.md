# ADR 0010 · Asambleas y votaciones

- **Estado:** aceptada
- **Fecha:** 2026-11-08
- **Reemplaza:** el invariante 4 de AGENTS.md §9 («secreto del voto») y el esquema de votación
  inicial (`inscripciones`, un bien = una persona).

## Contexto
El plan 06 cubre el ciclo completo de la asamblea general de propietarios (Ley 675 de 2001,
arts. 37–49): convocatoria, registro y poderes, quórum, orden del día, votaciones ponderadas por
coeficiente (presencial, web y WhatsApp), elecciones, consulta por escrito y acta. La práctica
de las PH (versión usada en 2026, `dad_phback`) mostró que el registro en la puerta es el cuello
de botella, que una persona representa varios bienes y que la plataforma maneja varias
asambleas a la vez con las mismas líneas de WhatsApp.

## Decisión
**Modelo de votante**
- **Padrón** por reunión: foto de los bienes activos con el coeficiente del cuadro vigente y su
  representante, tomada al convocar. Los coeficientes no cambian con una reunión instalada.
- **Asistente** = la persona presente con sus medios (teléfono, línea de WhatsApp, rótulo,
  papeleta) y los bienes que representa (`asistente_bienes`, un bien en un solo asistente a la
  vez; propios o por poder). **Un voto del asistente = una participación por cada bien**, en una
  transacción; no hay voto diferenciado por bien.
- La **asistencia la certifica la mesa** al firmar: cuenta para el quórum de inmediato. Salidas
  con hora y motivo (`asistente_presencias`) y reingresos; quien sale deja de votar.
- Las preguntas se votan con los bienes **habilitados** al abrirlas (presentes y sin
  restricción por mora), más los que ingresan mientras siguen abiertas
  (`pregunta_habilitados`): es la base de la decisión y el trigger la exige.
- **Morosos**: listado que entrega la PH por asamblea (sin depender de contabilidad), con
  efecto `informativo`, `sin_voto` o `sin_voto_ni_quorum`. **Validar con asesor** (la
  jurisprudencia limita restringir el voto del moroso); la elección queda auditada y en el acta.
- **Poderes**: máximo de bienes por apoderado, cargos que no pueden ser apoderados y soporte
  obligatorio son configuración de cada asamblea; la verificación puede quedar pendiente.

**Visibilidad del voto (cambia el invariante 4)**
- No existe el secreto absoluto en la práctica de las PH: se elimina `secreto`. Quedan
  `nominal` (el detalle va en el acta) y `reservado` (el vínculo bien → opción se guarda; el
  detalle solo con `asambleas.votos.ver_detalle` y cada consulta se audita). `VotoId` sigue en
  UUID v4: la hora ya está en la participación y no hay motivo para cambiarlo.
- La auditoría y las confirmaciones nunca llevan la opción elegida.

**Cómputo**
- Cada pregunta declara su base (`coeficiente`, por defecto, o `unidades`); el escrutinio
  calcula ambas y decide una; por unidades se advierte en el acta (art. 49).
- Reglas: simple (más de la mitad de lo habilitado, art. 45), calificada 70 % del total
  (art. 46) o personalizada (porcentaje sobre presentes o total, según el reglamento).
- Tipos: sí/no (con abstención opcional), selección única, selección múltiple (cada opción se
  decide por separado) y **elección por cociente electoral** con residuos mayores (votos en
  blanco cuentan para el cociente; un empate en el último cupo queda marcado). Las elecciones
  de consejo, revisor fiscal y comité crean las asignaciones de rol (origen `asamblea`) cuando
  el conductor las aplica.
- Quórum: más de la mitad en primera convocatoria; en segunda y por derecho propio, número
  plural (al menos dos bienes). Segunda convocatoria: tercer día hábil siguiente a las 8:00
  p. m. (art. 41); derecho propio: primer día hábil del cuarto mes tras el periodo (art. 39).
  Antelación de 15 días calendario en ordinarias de primera convocatoria.
- Fotos del quórum en la instalación y al abrir y cerrar cada pregunta (acta e impugnación).

**Canales**
- **Registro asistido en la mesa** (≤ 20 s por asistente): busca por código o documento,
  agrega con un toque los otros bienes y los poderes, entrega un **rótulo genérico Code 128**
  (paquetes preimpresos, hoja en PDF) y envía la plantilla `confirmar_asistencia` con un botón
  «Confirmar» que abre la ventana de 24 h de Meta. La mesa ve si la plantilla falló (acuses
  `statuses`) para entregar el rótulo.
- **WhatsApp**: cada botón o fila lleva `vt:{tenant}:{pregunta}:{opcion}`: dos asambleas
  simultáneas nunca mezclan votos. El texto libre solo vota con una única pregunta pendiente.
- El celular que digita la mesa queda solo en el asistente de esa asamblea; no se guarda en el
  censo (un apoderado puede ser alguien que no vuelve: su número no sirve para otra cosa).
- **Puestos de escaneo** (rótulo → opción) y **voto asistido** por operador: exigen
  `asambleas.votos.registrar`; clave de idempotencia por lectura (la consola reintenta tras un
  corte). Sin servidor local en el salón: internet con módem 4G de respaldo.
- **Papeleta virtual** (enlace personal, token como hash): ingreso en reuniones virtuales o
  mixtas, voto web y salida. El video va en un proveedor externo (Google Meet).
- **Consulta por escrito** (art. 43): al convocar se habilita a cada representante y las
  preguntas quedan abiertas hasta el plazo (máximo un mes); el worker cierra al vencer.

**Líneas de WhatsApp**
- Cualquier línea atiende cualquier PH. Más líneas suman rendimiento (80 mps cada una),
  resiliencia (calidad propia) y franquicia de mensajes de servicio gratuitos (1 000 al mes por
  número desde el 1-oct-2026); no suben el límite de clientes por día (es del portafolio).
- Una sola app de Meta suscrita a la cuenta de WhatsApp Business (WABA) recibe los webhooks de
  todos sus números (cada uno llega con su `phone_number_id`) y envía por cualquiera con el
  mismo token: basta un `WHATSAPP_ACCESS_TOKEN` (usuario del sistema) mientras todas las líneas
  estén en la misma WABA. El portafolio empieza con 2 números y sube a 20 al verificar el
  negocio (confirmar en la cuenta).
- La conversación iniciada por la plataforma (plantillas de asambleas y avisos de tickets) sale
  por la línea activa con más **franquicia de servicio** libre este mes; las plantillas se
  pagan aparte y no la consumen (`elegir_linea_whatsapp`). Queda fija en el asistente; si
  escribe por otra, se le responde por esa. `phone_number_quality_update` degrada la línea.
- Costo por asamblea: los mensajes de la reunión se marcan en el outbox (`reunion_id`) y se
  valorizan con `tarifas_whatsapp` (editable por la plataforma; el valor exacto sale de la
  factura de Meta). Se cobra dentro del valor de la asamblea.

**Tiempo real y acta**
- Tablero en vivo por WebSocket: cada cambio hace `pg_notify` en su transacción; la API escucha
  (`LISTEN`) y recalcula desde la BD a lo sumo cada ~500 ms por reunión con pantallas
  conectadas. Sin Redis (se retira de la infraestructura).
- Acta en PDF con **Typst como librería** (`infra-pdf`), plantilla versionada y datos ya
  calculados; fuentes embebidas. Sin firma electrónica: se sube el escaneo firmado, la comisión
  la aprueba y se publica (libro de actas).

## Consecuencias
- El esquema de votación inicial se reemplaza con una migración reversible (no tenía datos
  reales); `participaciones` pierde `UNIQUE (wamid)` (un voto cubre varios bienes).
- Nuevos permisos `asambleas.registrar`, `asambleas.votos.registrar` y
  `plataforma.whatsapp.gestionar`, y el rol plantilla `operador_asamblea`.
- Las plantillas `confirmar_asistencia` y `convocatoria_asamblea` deben aprobarse en Meta.
- Typst arrastra avisos RustSec transitivos (crates sin mantenimiento y DoS de `quick-xml`, que
  solo se alcanza si una plantilla analiza XML): quedan como excepciones documentadas en
  `deny.toml` (aprobado el 2026-11-08) y se revisan al actualizar Typst.
- Pendiente para una fase posterior: tarjetas de respuesta por imagen (tipo Plickers), prueba de
  carga con un simulador externo y la cola local de la consola de mesa (lado front).
