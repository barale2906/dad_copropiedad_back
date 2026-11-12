# ADR 0014 · Búsqueda de proveedores, contacto autorizado y enlace para los grupos

- **Estado:** aceptada
- **Fecha:** 2026-11-11
- **Plan:** `docs/planes/08-publicidad-local.md` (fase 2, sección 3)

## Contexto
En los grupos de WhatsApp de los conjuntos los residentes preguntan «¿alguien conoce una
costurera?». La plataforma **no entra a los grupos**: la Groups API de Meta admite 8
participantes y no permite unirse a grupos existentes, y automatizar un número normal viola los
términos de Meta y pondría en riesgo las líneas que llevan votaciones y avisos. La fase 2 lleva
esa pregunta al chat de la PH y a la app, sobre el directorio de la fase 1 (ADR 0013).

## Decisión
- **Búsqueda iniciada por el residente** (`casos_uso::busqueda_proveedores`): opción «Buscar un
  proveedor» del menú, o cualquier texto que empiece por «busco» / «necesito» (lo que trae el
  enlace de los grupos), y `POST …/directorio/busquedas` en la app. Reglas puras en
  `domain::directorio::busqueda`: se quitan las palabras de intención y cortesía
  (`consulta_busqueda`), se reconoce la categoría por nombre o sinónimo tolerando plurales
  (`resolver_categoria`; ambiguo → búsqueda por texto) y se sugieren hasta 3 fichas que alcanzan
  a la PH (`comparar_sugerencias`: destacada solo con el módulo `publicidad`, calificación,
  cercanía). Es una conversación de servicio: no exige la finalidad `publicidad` ni usa
  plantillas de marketing.
- **Registro** en `solicitudes_proveedor` (por tenant, RLS), idempotente por `wamid` o por
  petición. El texto es `dato_personal` y se borra a los **90 días**
  (`purgar_solicitudes_proveedor`, SECURITY DEFINER, worker de gestión); la fila queda con la
  categoría, los sugeridos y la autorización como prueba.
- **Contacto autorizado (lead)**: nada sale hacia el proveedor sin un «Sí» explícito para esa
  solicitud (`autorizado_en`). Entonces se encola, por cada sugerido con WhatsApp, la plantilla
  de utilidad `contacto_proveedor` (nombre comercial, nombre del residente, lo que busca y su
  celular del censo), prioridad de evento y `outbox.concepto = 'directorio'`. Cada envío queda
  en `contactos_proveedor` (append-only: prueba de la autorización y base del cobro `por_lead`
  de la fase 4). Repetir el «Sí» no reenvía.
- **«¿Le sirvió?»**: con el contacto se programa (outbox `disponible_en` = +7 días) la
  plantilla `opinion_proveedor` al residente, con el botón `op:{tenant}:{solicitud}`. El chat
  pide elegir el proveedor y la calificación (1 a 5) y la guarda con `directorio::resenar`:
  sigue exigiendo cuenta, permiso `directorio.resenar` y rol de propietario o residente
  derivado vigente (reseñas verificadas, ADR 0013).
- **Enlace público para los grupos** (`enlaces_directorio`, uno por PH, solo el SHA-256 del
  token): la administración lo genera, consulta o revoca con el permiso nuevo
  `directorio.compartir` (administrador y asistente). `GET …/directorio/publico` no tiene
  sesión: el token va en `X-Enlace-Directorio`, con `LimitePublico`; devuelve las mismas fichas
  que ve un residente (sin texto de reseñas) y el enlace
  `https://wa.me/<línea>?text=Busco%20`. El número es el de la primera línea activa
  (`numero_whatsapp_publico`), no el de la rotación: un enlace pegado en un grupo no debe
  cambiar; cualquier línea atiende cualquier PH.

## Consecuencias
- Plantillas por aprobar en Meta (categoría utilidad): `contacto_proveedor` (4 parámetros) y
  `opinion_proveedor` (2 parámetros y botón de respuesta rápida «Calificar»).
- El proveedor recibe el contacto por WhatsApp aunque aún no exista su portal; la fase 3 le
  mostrará sus contactos (`contactos_proveedor`) y la fase 4 los liquidará `por_lead`.
- Generar un enlace nuevo invalida el anterior (hay que volver a pegarlo en los grupos): el
  token no se puede volver a consultar.
- Quien escribe desde el enlace sin estar en el censo recibe el mensaje habitual de número no
  registrado; la página pública le sirve igual.
