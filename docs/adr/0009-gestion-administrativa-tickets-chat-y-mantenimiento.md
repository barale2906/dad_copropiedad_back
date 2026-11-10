# ADR 0009 · Gestión administrativa: tickets, chat de WhatsApp, inventario y mantenimiento

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Contexto
La administración atiende peticiones, quejas, reclamos y solicitudes por varios canales; los
residentes quieren reportar daños y consultar la PH por WhatsApp; los equipos de la copropiedad
necesitan hoja de vida, mantenimiento preventivo y control de certificaciones (planes 07b y
07c). Todo es parte del módulo vendible `gestion_administrativa`.

## Decisión
**Tickets**
- Número legible **por copropiedad y año**, una sola serie para todos los tipos
  (`PQR-2026-00042`), con la tabla `consecutivos` (`INSERT … ON CONFLICT DO UPDATE … RETURNING`,
  atómico aun con radicaciones simultáneas). Las órdenes de trabajo usan la misma tabla
  (`OT-2026-00007`).
- Estados solo por `domain::EstadoTicket::transicionar`; cada cambio deja un evento en
  `ticket_eventos` (append-only). Quién puede qué depende de su papel: gestiona
  (`tickets.asignar`), atiende (asignado a su cuenta o a la organización con contrato para la
  que trabaja), o es el solicitante (cierra o reabre).
- **Plazos** en horas corridas, horas hábiles o días hábiles con los festivos de Colombia y la
  jornada de cada PH (`domain::ticket::vencimiento`). Valores por defecto: Ley 1755 de 2015
  (15 días hábiles para peticiones, 10 para documentos) y mantenimiento por prioridad (urgente
  4 horas hábiles, alta 2, media 5, baja 10 días hábiles); cada PH los cambia por tipo y
  prioridad. Mientras se espera al solicitante el plazo no corre y al responder se cuenta de
  nuevo (art. 17).
- **Reapertura** dentro de 5 días hábiles (configurable); después el worker lo cierra e invita
  a la encuesta (1 a 5).
- **Convivencia confidencial**: la identidad del denunciante y su bien privado solo los ve
  quien tiene `tickets.convivencia.ver` (plantilla del comité, art. 58); tampoco se puede
  filtrar por bien privado para deducirla.
- Notas internas que el solicitante nunca ve (`ticket_mensajes.interno`).
- **Urgencias**: se avisa de inmediato, por correo y plantilla de WhatsApp, a las cuentas con
  `tickets.urgencias.recibir` (función `SECURITY DEFINER` `destinatarios_con_permiso`).

**Chat de WhatsApp**
- **Cualquier línea atiende cualquier PH**: la PH se resuelve por el teléfono inscrito como
  titular o residente (función `vinculos_por_telefono`), y si hay varios vínculos se pregunta con
  una lista.
- **Sesiones en PostgreSQL**, no en Redis (decisión del 2026-11-08): tabla de plataforma
  `sesiones_chat` con vencimiento. Cada turno se cierra en **una transacción** que guarda la
  sesión, encola las respuestas en el outbox y marca el `wamid` (`mensajes_entrantes`); los
  casos de uso que invoca son idempotentes por `wamid`, así un reintento de Meta no duplica nada.
  Redis queda para el pub/sub de resultados de las asambleas (plan 06).
- Máquina de estados en `application::casos_uso::chat` (menú, preguntas frecuentes, reportar
  daño con foto, mis solicitudes, reservas, parqueadero de visitantes, citas, bandeja humana);
  el gateway solo traduce el webhook. Reservar exige cuenta en la plataforma: se usan sus
  permisos como en la web.
- Respuestas libres solo dentro de la **ventana de 24 h** de Meta; fuera de ella, plantillas de
  utilidad aprobadas (`ticket_actualizado`, `reporte_urgente`) que deben registrarse en Meta.
- Cliente de Cloud API propio en `infra-whatsapp` con `reqwest` (rustls + ring + webpki-roots,
  sin OpenSSL ni aws-lc). El ritmo de 80 mensajes/s por línea lo aplica el gateway
  (`mensajeria::Ritmo`), sin agregar `governor`.

**Inventario y mantenimiento**
- Activos con código único, hoja de vida (archivos, movimientos append-only, órdenes y costo
  acumulado). La baja exige soporte y es definitiva.
- Planes preventivos con recurrencia (cada N días, semanas o meses desde una fecha; los meses se
  cuentan desde el inicio para no arrastrar el fin de mes). El worker de gestión genera las
  órdenes con anticipación; **una orden por (plan, fecha)** también en la BD
  (`ux_orden_por_periodo`).
- Órdenes `programada → en_ejecucion → ejecutada → verificada` (o `cancelada`); quien ejecuta no
  verifica. Verificar una correctiva puede resolver el ticket que la originó y avisar al
  solicitante. Las empresas externas operan las órdenes asignadas a su organización.
- Obligaciones periódicas con un **catálogo sugerido editable** (ascensores, tanques,
  extintores, póliza de bienes comunes del art. 15, plan de emergencias…), sin citar normas
  locales: la PH confirma la periodicidad que le exige su municipio.

## Consecuencias
- Se agregan las dependencias `reqwest`, `rustls` y `webpki-roots` (licencias permitidas).
- El gateway necesita las variables `ARCHIVOS_*` (guarda las fotos que llegan por WhatsApp) y,
  para enviar, `WHATSAPP_ACCESS_TOKEN`; sin token recibe pero no envía.
- Hay que registrar en Meta las plantillas `ticket_actualizado` (número, estado, detalle) y
  `reporte_urgente` (número, asunto, ubicación) en español.
- Actualización 2026-11-10 (plan 10, fase 1): el menú del chat ofrece «Autorizar una visita»
  (QR temporal sin exigir cuenta: el teléfono identifica a la persona), «Mis paquetes» y, con una
  asamblea en curso, «Votar» (reenvía las preguntas abiertas pendientes). `GET /api/v1/yo/tickets`
  reúne las solicitudes de la cuenta en todas sus PH (función `tickets_de_cuenta`). Las
  solicitudes del titular de datos se atienden en su propio módulo (ADR 0004). El menú prioriza
  las opciones porque WhatsApp admite 10 filas.
