# ADR 0007 · Censo: residentes, vehículos, mascotas y autoservicio con aprobación

- **Estado:** aceptada
- **Fecha:** 2026-11-07

## Contexto
La Ley 675 (art. 51) obliga al administrador a llevar el registro de propietarios y residentes.
Los planes de bienes comunes (04), control de acceso (05) y asambleas (06) necesitan saber quién
vive en cada bien privado, qué vehículos y mascotas tiene y quién responde ante la PH.

## Decisión
- **Libro de propietarios** = consulta sobre `titularidades` (plan 03) a una fecha de corte, con
  el coeficiente del cuadro que regía entonces. No hay tabla nueva. **Libro de residentes** =
  consulta sobre `residencias`. Ambos en JSON o CSV (`;`, UTF-8 con BOM, para Excel); el PDF
  llegará con el adaptador de Typst (`infra-pdf`).
- **Residencias** con calidad (propietario residente, tenedor, familiar, huésped temporal,
  empleado doméstico), vigencia y a lo sumo un **responsable** vigente por bien (índice único
  parcial). Un propietario residente es titular vigente; un tenedor lo autoriza un titular
  vigente. Propietarios residentes, tenedores y familiares con cuenta reciben el rol derivado
  `residente` (origen `residencia`), que vence en la misma transacción que termina la residencia.
- **Vehículos** con placa normalizada (`ABC123`), única entre los vigentes de la PH; carros y
  motos siempre con placa. **Mascotas** con vacunas, póliza (razas potencialmente peligrosas,
  Ley 1801) y estado (activa, fallecida, trasladada). Las faltas (vacunas o pólizas vencidas o
  por vencer en 30 días, póliza ausente) son **alertas**, no bloqueos: el registro refleja la
  realidad aunque incumpla.
- **Personas**: tipo de documento (incluye TI y RC para menores), fecha de nacimiento,
  contacto de emergencia y movilidad reducida. La fecha de nacimiento y la movilidad son datos
  **sensibles**: solo se ven y editan con `personas.datos_sensibles.ver`; quien no lo tiene ve
  `es_menor`. Se alertan los residentes sin autorización de tratamiento para administración.
- **Autoservicio**: un titular o residente vigente propone residencias, vehículos o mascotas de
  su bien (`cambios_censo`, datos en JSON); la administración aprueba (se crea el registro en la
  misma transacción) o rechaza con motivo. Una propuesta se resuelve una sola vez (trigger) y
  nunca vincula cuentas de la plataforma.
- `RepositorioPersonas` se separa de `RepositorioTitulares`: lo usan titulares, importaciones y
  el censo.

## Consecuencias
- Portería (plan 05) y parqueaderos (plan 04) consultan vehículos por placa y residentes por
  bien; ese plan decide qué ve el rol `vigilante` (hoy no tiene `censo.ver`).
- Las campañas periódicas de actualización por WhatsApp quedan para la fase de autoservicio del
  plan 07 (necesitan plantillas aprobadas por Meta y el envío del gateway).
