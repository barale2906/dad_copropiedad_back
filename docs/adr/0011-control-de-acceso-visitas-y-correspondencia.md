# ADR 0011 · Control de acceso, visitas y correspondencia

- **Estado:** aceptada
- **Fecha:** 2026-11-09
- **Plan:** `docs/planes/05-control-de-acceso.md` (fases 1–5)

## Contexto
La portería de una PH colombiana la presta casi siempre una empresa de seguridad contratada.
Hay que gestionar visitas (anunciadas en la portería o pre-autorizadas por un residente),
correspondencia que espera en la portería y, cuando haya hardware, torniquetes, talanqueras,
lectores, cámaras de placas y biometría. Aún no se sabe qué hardware tienen las PH objetivo, y
la portería no puede quedarse sin acceso si se cae internet. Los avisos por WhatsApp tienen
costo; las apps Android/iOS se harán en **Flutter** y reciben notificaciones push.

## Decisión
**Visitas (decisiones del usuario del 2026-11-09)**
- La portería anuncia: **aviso 1** al **primer respondiente** (el `responsable` del censo); si
  nadie responde en **2 min**, **aviso 2** a todos los residentes adultos; a los **5 min** la
  visita queda en `requiere_llamada` y la portería ve los teléfonos (cada consulta se audita) y
  registra el resultado de la llamada. Los domicilios siguen el mismo flujo.
- **Destinatarios** (`domain::acceso::destinatarios_aviso`): nunca menores de edad; los
  propietarios que no viven en el bien solo se avisan si **no vive allí ningún residente de 18
  a 59 años**: todos los adultos son personas mayores (60 años o más, Ley 1276 de 2009; los
  menores no cuentan) o no hay residentes adultos. Sin fecha de nacimiento se presume adulto
  menor de 60.
  Puede decidir quien recibiría el aviso de nivel 2.
- **La primera respuesta gana** (`FOR UPDATE`); las siguientes reciben «ya fue atendida».
- Respuestas por la app, la web o los botones de la plantilla `visita_anunciada`
  (`vis:{tenant}:{visita}:{a|r}`, idempotentes por `wamid`, verificando el teléfono).
- **Pre-autorización** con QR temporal firmado **Ed25519** (`DQ1.<carga>.<firma>`): ventana,
  usos y visita, sin datos personales. La clave de cada PH se **deriva** de
  `ACCESO_QR_SECRETO` con HMAC-SHA256 sobre el tenant: no hay claves privadas que guardar; el
  puente recibe la pública y valida sin internet.

**Correspondencia**
- Aviso al recibir y **cada 24 h** hasta la entrega (el primero al primer respondiente, los
  siguientes a todos), respetando un **horario de silencio** de la PH (21:00–07:00 por
  defecto) y un **tope de días** (7 por defecto); al agotarse se alerta una vez a la
  administración (`acceso.configurar`).
- Entrega con **firma o foto** (al menos una) de quien recoge, elegido del listado del bien o
  anotado como tercero con documento. La firma y la foto se guardan **1 año por defecto,
  editable por cada PH** (30–3650 días); la purga borra los bytes y lo deja anotado
  (`purgas_soportes_entrega`), la entrega sigue siendo append-only.

**Canal de los avisos**
- **Push primero, WhatsApp de respaldo** (`elegir_canal_aviso`): el outbox tiene el canal
  `push` (`MensajeOutbox::Push`) con el WhatsApp de respaldo dentro del mensaje. Si FCM dice que
  el token no existe, el worker lo desactiva y encola el respaldo en la misma transacción
  (`outbox_desviar_push`), sin reintentar.
- Push con **Firebase Cloud Messaging HTTP v1** (crate `infra-push`, puerto `EnvioPush`):
  sirve para Android e iOS desde Flutter. Autenticación con una cuenta de servicio
  (`FCM_CREDENCIALES`): JWT RS256 firmado con `ring`, token de acceso de una hora en caché.
- Los avisos de acceso se marcan en el outbox (`concepto = 'control_acceso'`) y
  `/acceso/costo-avisos` los valoriza con la tarifa de utilidad: **la cuenta se lleva por PH**.

**Credenciales, puentes y dispositivos**
- Credenciales por **huella** SHA-256 de `tenant:tipo:identificador normalizado`; nunca el PIN,
  la tarjeta ni el texto del QR en claro. Sin dos credenciales vigentes con la misma huella.
- **Autorizaciones derivadas calculadas, no guardadas** (`ventanas_acceso`): residencia o
  titularidad vigente, vehículo de un bien con vinculados, reserva con `acceso_automatico`.
  Los triggers suben la versión de sincronización de las credenciales afectadas cuando cambia el
  censo, la titularidad, una autorización o una reserva: terminar la residencia o vender el bien
  retira el acceso en la siguiente sincronización (≤ 20 s por defecto).
- **Puente** (`apps/puente-acceso`, binario `dad_puente_acceso`): decide en el borde con una
  cache en disco, cuenta los usos de los QR localmente, guarda los eventos en una cola en disco
  y los reporta idempotentes por `id_local`. Se autentica con un **token de máquina** (en BD solo
  su SHA-256), rotable y revocable. Adaptadores por fabricante tras `ControladorDispositivo`; por
  ahora solo el simulado (el primero se implementa con el hardware de la primera PH). La
  configuración de un dispositivo nunca lleva secretos.
- **Eventos** append-only y **particionados por mes** (UTC); un trigger solo permite borrar los
  de más de un año, y la función `purgar_eventos_acceso` aplica el plazo de la PH (1–2 años).
  Fotos de placas: 30 días por defecto (guía de la SIC).
- **Biometría**: solo con consentimiento explícito vigente (`consentimientos_biometricos`), con
  método alterno; la plantilla nunca sale del equipo. Revocar el consentimiento revoca las
  credenciales y el puente borra la plantilla y lo confirma.

**Permisos nuevos**: `acceso.correspondencia.entregar`, `acceso.minuta.registrar|ver`,
`acceso.configurar`, `acceso.credenciales.gestionar`, `acceso.biometria.enrolar` (vigilante y
supervisor de seguridad reciben los de portería).

## Consecuencias
- La API exige `ACCESO_QR_SECRETO` para arrancar; rotarlo invalida los QR emitidos.
- Sin `FCM_CREDENCIALES` el gateway no envía push y todo sale por WhatsApp (con costo).
- Hay que aprobar en Meta las plantillas `visita_anunciada` (botones «Autorizar» /
  «Rechazar»), `correspondencia_recibida` y `correspondencia_entregada`.
- El worker de acceso corre cada 15 s en el gateway; la retención, una vez al día, y prepara
  las particiones de los próximos meses.
- Pendientes: adaptadores reales de hardware (cuando se conozca el de la primera PH), envío del
  QR al visitante por WhatsApp como imagen, autorizar visitas desde el chat (plan 07b) y alertas
  push a la portería.
