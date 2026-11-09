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
