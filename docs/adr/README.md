# Registro de decisiones de arquitectura (ADR)

Cada decisión relevante (tecnología, patrón, cambio de estructura) se registra en un archivo
`NNNN-titulo-corto.md` con: **Estado**, **Fecha**, **Contexto**, **Decisión** y **Consecuencias**.
Los ADR no se reescriben: si una decisión cambia, se crea uno nuevo que reemplaza al anterior.

| # | Decisión | Estado |
|---|---|---|
| [0001](0001-arquitectura-hexagonal-multitenant.md) | Monolito modular hexagonal, multi-tenant con RLS | Aceptada |
| [0002](0002-terminologia-ley-675.md) | Términos de la Ley 675 de 2001 en todo el sistema | Aceptada |
| [0003](0003-almacenamiento-de-archivos.md) | Archivos en el disco del VPS, listos para migrar a S3 | Aceptada |
| [0004](0004-proteccion-de-datos-personales.md) | Protección de datos personales por finalidad (Ley 1581) | Aceptada |
| [0005](0005-identidad-roles-y-permisos.md) | Identidad global, roles por ámbito y sesiones opacas | Aceptada |
| [0006](0006-copropiedades-coeficientes-y-titulares.md) | Coeficientes versionados, titulares e importación masiva | Aceptada |
| [0007](0007-censo-residentes-vehiculos-mascotas.md) | Censo: residentes, vehículos, mascotas y autoservicio | Aceptada |
| [0008](0008-bienes-comunes-reservas-y-sorteos.md) | Bienes comunes, motor de reservas, parqueaderos y sorteos verificables | Aceptada |
| [0009](0009-gestion-administrativa-tickets-chat-y-mantenimiento.md) | Tickets con plazos hábiles, chat de WhatsApp con sesiones en PostgreSQL, inventario y mantenimiento | Aceptada |
| [0010](0010-asambleas-y-votaciones.md) | Asambleas: asistente que vota por todos sus bienes, voto nominal o reservado, cociente electoral, mesa asistida, WhatsApp multilínea, tiempo real con LISTEN/NOTIFY y actas con Typst | Aceptada |
| [0011](0011-control-de-acceso-visitas-y-correspondencia.md) | Control de acceso: avisos push primero con escalamiento, correspondencia con firma o foto, QR Ed25519, credenciales por huella, autorizaciones derivadas y puente en sitio | Aceptada |
| [0012](0012-cierre-para-produccion.md) | Cierre para producción: límite de peticiones propio por IP, consulta de auditoría y despliegue con Docker y GHCR | Aceptada |
| [0013](0013-directorio-de-comercios-y-servicios.md) | Directorio de comercios y servicios: datos de plataforma, cobertura por radio/municipio/país sin datos de personas, destacadas solo con el módulo `publicidad`, reseñas anónimas por PH con promedio global y moderación posterior | Aceptada |
| [0014](0014-busqueda-de-proveedores-y-contacto-autorizado.md) | Búsqueda de proveedores en el chat y la app, contacto al proveedor solo con «Sí» explícito por solicitud, «¿Le sirvió?» a los 7 días, texto purgado a los 90 días y enlace público para los grupos de WhatsApp | Aceptada |
| [0015](0015-publicidad-de-la-plataforma.md) | Publicidad de la plataforma: autorización del titular frente a la plataforma (la PH no la da), portal del anunciante, moderación siempre, segmentación por perfil de la PH, topes de presupuesto y frecuencia, masivos con prioridad 9, cupones y liquidación sin reparto a las PH | Aceptada |
| [0016](0016-entidad-generica-y-verticales.md) | Entidad genérica como tenant (la copropiedad es su extensión 1:1 con el mismo id); cada módulo se activa uno a uno y los de la PH solo en una copropiedad | Aceptada |
| [0017](0017-nucleo-contable.md) | Núcleo contable genérico: partida doble como tipo y en la BD, comprobantes inmutables con reverso, periodos con reapertura aprobada por otra cuenta, contabilizador de documentos origen, terceros = personas, PUC y plantillas editables | Aceptada |
| [0018](0018-tesoreria-y-cuentas-por-pagar.md) | Tesorería, cuentas por pagar, facturas de venta y conciliación: documentos por eventos del contabilizador con la cuenta del documento, anular = revertir, un solo recaudo con `aplicaciones_pago`, egresos autorizados por otra cuenta, retenciones con tarifa vigente, extractos (planilla y Asobancaria) sin duplicar y conciliación append-only | Aceptada |
| [0019](0019-contabilidad-propiedad-horizontal.md) | Vertical contable de la PH (`contabilidad_ph`): conceptos de cobro y política versionada, módulos de contribución como cuadros de coeficientes, presupuesto, liquidación con mayor residuo y cuentas de cobro inmutables, recaudo = recibo con aplicaciones a obligaciones (saldo reconstruido), pronto pago, anticipos, cartera inicial y cuadre contra el mayor | Aceptada |
