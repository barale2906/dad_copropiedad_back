# ADR 0016 · Entidad genérica y verticales

- **Estado:** aceptada
- **Fecha:** 2026-11-12 (corregida el 2026-11-13: cada módulo se activa uno a uno)
- **Plan:** `docs/planes/12-contabilidad-nucleo.md` (fase 1); análisis en `docs/planes/11-…`
- **Actualiza:** ADR 0001 (el tenant) y ADR 0002 (glosario: tenant = copropiedad)

## Contexto
El módulo contable se venderá a **cualquier empresa** y cada cliente tendrá la vertical de su
actividad (PH primero; gimnasios, tiendas, repuestos después). Hasta ahora el tenant era la
copropiedad: la tabla `copropiedades` era a la vez el tenant y los datos de la Ley 675, con 107
llaves foráneas apuntándole. Una empresa que no es PH no cabía. Además, una PH puede contratar
solo un módulo (p. ej. asambleas): ningún "paquete" de la vertical debe ser obligatorio.

## Decisión
- **El tenant es la entidad.** `copropiedades` se renombra a `entidades` (PostgreSQL conserva las
  llaves foráneas, la política RLS y los permisos por OID). `app_tenant_id()`, `TenantId`,
  `UnidadDeTrabajo` y RLS no cambian: el id del tenant es el de la entidad.
- **La copropiedad es una extensión 1:1** con el **mismo id** (`copropiedades.id → entidades.id`)
  y solo las columnas propias de la PH: tipo, forma, estrato, escritura, notaría, matrícula
  matriz, reglamento, certificado y nomenclatura. Lo genérico (nombre, razón social, NIT,
  ubicación, contacto, zona horaria, estado) queda en `entidades`. La ficha de la API no cambia.
- **Una entidad es PH porque tiene fila en `copropiedades`**; la vertical no es un módulo
  contratable. Lo básico de la PH (copropiedad, bienes privados, censo) viene con ella y **cada
  módulo vendible se habilita uno a uno** (una PH puede tener solo asambleas).
- **Las tablas propias de la PH apuntan a `copropiedades (id)`** (bienes privados, censo,
  asambleas, reservas, acceso, tickets, comunicados, publicidad en la PH…): la base de datos
  impide que una entidad que no es PH tenga esos datos. Las **transversales** siguen en
  `entidades`: roles y asignaciones, módulos, archivos, auditoría, outbox, personas, protección de
  datos, consecutivos, contratos, importaciones y organizaciones.
- **Módulos de la vertical**: `Modulo::exige_copropiedad()`; habilitarlos en una entidad que no
  es copropiedad responde 404. `contabilidad` sirve a cualquier entidad.
- **Rutas**: `/copropiedades/{copropiedad_id}/…` (extractor `EnCopropiedad`, que además exige que
  la entidad sea copropiedad: `exigir_copropiedad`, 404 si no) y `/entidades/{entidad_id}/…`
  (extractor `EnEntidad`, cualquier entidad: capacidades y contabilidad).
- **Alta**: la plataforma da de alta una copropiedad (entidad + extensión) o una entidad sin
  vertical (`POST /plataforma/entidades`, permiso `plataforma.entidades.crear`), siempre con su
  primer administrador y sin módulos.
- Las funciones SQL que leían `copropiedades` por nombre se recrearon sobre `entidades` (las
  LANGUAGE sql guardan el texto); `campana_alcanza` toma tipo y estrato de la extensión.

## Consecuencias
- Los nombres de Rust de la vertical (`casos_uso::copropiedades`, `EnCopropiedad`,
  `FichaCopropiedad`) no cambian: son de la PH. `puertos::personas` re-exporta el puerto de
  personas para los módulos genéricos.
- Las rutas, los ids y la ficha de la API no cambian para el frontend.
- La reversión (down) devuelve el esquema anterior con los datos de las PH; una entidad que no sea
  PH no cabe en él y se perdería.
- Una vertical nueva = su tabla de extensión 1:1 + sus módulos con su `exige_…` + sus rutas. Si
  una vertical llega a necesitar dependencias entre módulos (p. ej. la contabilidad de la PH
  sobre la base), se agregan en `Modulo` cuando existan.
- Las rutas de roles, archivos y auditoría siguen bajo `/copropiedades/{id}`; las entidades sin
  vertical se administran por ahora con su administrador inicial (se generalizan cuando una
  vertical nueva lo pida).
