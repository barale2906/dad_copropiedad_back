# ADR 0015 · Publicidad de la plataforma: portal, moderación, entrega y liquidación

- **Estado:** aceptada
- **Fecha:** 2026-11-11
- **Plan:** `docs/planes/08-publicidad-local.md` (fases 3 a 6)

## Contexto
La publicidad es un negocio **de la plataforma**. El 2026-11-11 el usuario lo precisó: las
copropiedades no tienen nada que ver con los anuncios; los anuncios llegan a los residentes que
**autoricen** recibirlos, y la PH es solo el medio para llegar a esos posibles clientes. Con esa
precisión se ejecutaron las fases 3 a 6 (la 7, pagos y facturación electrónica, espera el
módulo de contabilidad).

## Decisión
- **Autorización frente a la plataforma.** La finalidad `publicidad` se otorga bajo la política
  de la plataforma (`politicas_publicidad`, versionada, la publica `plataforma.publicidad.liquidar`),
  no bajo la de la PH: `autorizaciones_tratamiento.politica_publicidad_id` (y `politica_id` nulo).
  Solo el titular la otorga (`POST …/publicidad/autorizacion`, aceptando la versión vigente);
  `registrar_autorizacion` de la PH rechaza la finalidad `publicidad`. Revocar: la app, o el
  botón «No recibir más» (`baja:{tenant}`) de cada mensaje de WhatsApp.
- **Sin autorización no hay publicidad**: ni piezas en la app ni mensajes masivos. Las pantallas
  del lobby son una cartelera pública (no tratan datos de nadie) y solo dependen del módulo.
- **Datos de plataforma** (RLS `solo_plataforma`): `campanas` (con la segmentación en columnas),
  `campana_copropiedades`, `creativos`, `tarifas_publicidad`, `entregas_publicidad` (agregadas
  por creativo, PH, canal y día: nunca personas) y `liquidaciones` (**sin reparto a las PH**).
  Por copropiedad: `envios_publicidad` (tope de frecuencia), `reportes_publicidad` y
  `pantallas_publicidad`. Lo que cruza ámbitos va por funciones `SECURITY DEFINER`
  (`piezas_publicidad`, `pieza_para_entrega`, `aplicar_entrega_publicidad`,
  `conflicto_interes_campana`, `alcance_campana`, `leads_anunciante`,
  `consumo_directo_anunciantes`, `reportes_publicidad_pendientes`, `atender_reportes_publicidad`,
  `copropiedades_con_publicidad`).
- **Portal del anunciante** (`/api/v1/anunciantes`): cualquier cuenta se registra (organización
  nueva como dueña, o una que ya administra); queda `pendiente` hasta la verificación. Permisos
  por papel en la organización (`domain::publicidad::permite`): dueño y gestor todo; operario
  solo redime cupones. A quien no es miembro se le responde 404.
- **Moderación siempre**: creativos uno por uno (cambiar el contenido los devuelve a
  `pendiente`) y campañas (`en_revision → aprobada | rechazada`). Aprobar exige anunciante
  verificado, todos los creativos aprobados y una tarifa vigente del modelo, que queda fijada en
  la campaña. El conflicto de interés (contrato de servicio vigente en una PH alcanzada) se
  calcula al enviar y al moderar y queda marcado.
- **Segmentación solo con el perfil de la PH** (copropiedades elegidas, radio desde el
  anunciante, municipio, tipo, número de bienes, estrato): `Segmentacion::alcanza` en el dominio
  y `campana_alcanza` en SQL.
- **Cobro y topes**: `ModeloCobro` (`cpm`, `cpc`, `por_redencion`, `por_lead` por evento;
  `tarifa_fija` y `directorio_destacado` por periodo). Cada entrega bloquea la campaña
  (`FOR UPDATE`), se recorta a lo que cabe en el presupuesto total y diario
  (`ajustar_al_presupuesto`), se valora al registrarla y finaliza la campaña si el total ya no
  alcanza para un evento. Un cupón vigente se honra aunque ya no se pueda cobrar (Ley 1480).
- **Masivos** (worker del gateway, cada 15 min): plantillas de marketing aprobadas en Meta (el
  nombre va en el creativo) y boletines por correo; destinatarios con `destinatarios_comunicado`
  y la finalidad `publicidad` (nunca menores), una campaña por persona y un masivo por canal
  cada 7 días; prioridad 9 y `outbox.concepto = 'publicidad'` con `outbox.campana_id` (costo de
  WhatsApp por campaña). Cada mensaje cuenta como impresión.
- **Liquidación mensual** (`plataforma.publicidad.liquidar`, mes cerrado): consumo variable +
  cargos fijos de las campañas del periodo + contactos autorizados × tarifa `por_lead`; el costo
  de WhatsApp para la plataforma es informativo. `calculada → facturada → pagada`; solo una
  `calculada` se recalcula.

## Consecuencias
- Imágenes de los creativos, logos y documentos del anunciante siguen pendientes: falta un
  almacén de archivos de plataforma (los creativos son texto, botón y enlace `https://`).
- Las plantillas de marketing de cada campaña deben aprobarse en Meta antes de activarla.
- La pauta B2B a la administración de la PH (decisión del 2026-11-10) quedó fuera: contradice
  que los anuncios solo lleguen a residentes que autoricen. Se retoma si el usuario lo pide.
- Los cupones son códigos por oferta (con límite de usos), no uno por residente.
- Las autorizaciones de publicidad dadas antes bajo la política de una PH (solo en
  desarrollo) no cumplen el CHECK nuevo (`NOT VALID`); en producción no las hay.
