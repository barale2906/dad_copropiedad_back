# ADR 0025 · Nómina que se vende sola: catálogo laboral de plataforma, empleados y contratos versionados

- **Estado:** aceptada (fase 1)
- **Fecha:** 2026-11-20
- **Plan:** `docs/planes/16-nomina-electronica.md`

## Contexto
Cualquier entidad (una PH con su todero, un gimnasio, la misma GISLA) necesita registrar a sus
empleados, liquidar la nómina, preparar la PILA y transmitir la nómina electrónica a la DIAN. El
usuario decidió (2026-11-14) que la nómina **se vende sola** y se integra con la contabilidad si la
entidad la tiene, con la misma regla de la facturación (ADR 0022); periodicidad mensual y
quincenal; PILA solo como archivo plano al inicio; aprendices SENA desde la fase 1 y servicio
doméstico después; los cálculos se escriben con la norma conocida y el especialista laboral del
usuario los ajusta. La fase 1 cubre el catálogo, los empleados, los contratos y su carga masiva.

## Decisión
- **Módulo vendible `nomina`** (`Modulo::Nomina`), para cualquier entidad (no exige copropiedad
  ni contabilidad). Rutas `/entidades/{entidad_id}/nomina`. Reglas puras en `domain::nomina`,
  casos de uso en `casos_uso::nomina`, puerto `puertos::nomina`. **La contabilidad no importa la
  nómina** y la nómina no conoce a la PH (pruebas de fronteras en `domain` y `application`).
- **Catálogo laboral de plataforma**, nunca en el código:
  - `parametros_laborales`: un parámetro por variante de `ParametroLaboral` (salario mínimo,
    auxilio, jornada, recargos, aportes, ARL por clase, salario integral, incapacidades,
    prestaciones, apoyo del aprendiz…), cada uno con su `Unidad` (pesos, porcentaje, salarios
    mínimos, horas, hora del día, días) que acota su valor. **Solo inserción** con fecha y fuente:
    vale la versión de mayor `vigente_desde` hasta la fecha (no se guarda `vigente_hasta`, así una
    versión intermedia no deja huecos ni solapes). Un error se corrige con otra versión. Sin el
    parámetro vigente, la regla que lo necesita falla con un mensaje que lo nombra.
  - `administradoras_seguridad_social` (EPS, AFP, cesantías, cajas y ARL) con su código de la
    PILA, único por tipo. Es un catálogo regulado que mantiene la plataforma: no son
    organizaciones de los clientes (`organizaciones`), que las crean las entidades y no llevan
    código PILA. Una inactiva no se usa en contratos ni versiones nuevas.
  - Lo escribe `plataforma.normativa.gestionar` (el mismo permiso de los catálogos tributarios).
  - Los festivos salen de `domain::calendario` (ya existente), no del catálogo.
- **Empleado = persona de la entidad** (`empleados.persona_id`, la misma de los terceros y del
  censo; si el documento ya existe, se reutiliza). Una persona es empleada una sola vez por
  entidad; el empleado **nace con su primer contrato**. Su estado (`activo` / `retirado`) se
  deriva de los contratos.
- **Contrato y versiones**: `contratos_laborales` guarda lo que no cambia (tipo, inicio, fecha
  final pactada, obra) y la terminación con su motivo; `versiones_contrato_laboral` guarda las
  condiciones desde una fecha (cargo, salario, tipo de salario y etapa del aprendiz, jornada,
  periodicidad, afiliaciones, clase de riesgo, pensionado, medio de pago y cuenta). Modificar =
  versión nueva posterior a la última (solo inserción). El término fijo se prorroga moviendo la
  fecha final; terminar no se permite antes de la última versión. Nada se borra.
- **Reglas en el dominio** (`validar_condiciones`, contra el catálogo vigente en la fecha de la
  versión): aprendizaje ⇔ apoyo de sostenimiento con su etapa; salario ordinario ≥ mínimo
  (proporcional en jornada parcial), integral ≥ su mínimo en salarios mínimos, apoyo ≥ el
  porcentaje de su etapa; afiliaciones: EPS siempre, pensiones salvo pensionados y aprendices,
  cesantías solo con salario ordinario, caja salvo aprendices, ARL salvo el aprendiz en etapa
  lectiva. **Están marcadas para la revisión del especialista laboral.**
- **La BD vuelve a exigir** (código `V0010` → `ReglaLaboral`): contratos de un empleado sin
  cruces (`EXCLUDE` con `btree_gist` sobre el rango inicio–terminación o fin), primera versión el
  día del inicio (trigger diferido), versiones que avanzan y caen dentro del contrato,
  aprendizaje ⇔ apoyo, contrato terminado inmutable, tipo e inicio inmutables. `dad_app` no tiene
  `UPDATE` ni `DELETE` sobre versiones ni catálogo.
- **Datos personales**: salario, cuenta bancaria (completa: la necesitan la dispersión y la
  nómina electrónica), fondos y la condición de pensionado son `dato_personal`. Solo los ve
  `nomina.salarios.ver`: el caso de uso los devuelve vacíos a quien tiene solo `nomina.ver`, y el
  soporte de la plataforma nunca los recibe (`VEDADOS_A_SOPORTE`). La auditoría no los registra.
- **Permisos**: `nomina.ver`, `nomina.salarios.ver`, `nomina.empleados.gestionar` (plantillas
  `contador` y `administrador`).
- **Carga masiva**: tipo de importación `empleados` con el flujo de siempre (vista previa →
  confirmar en una transacción), planilla en `domain::importacion::empleados` (administradoras
  por código PILA). El lector de XLSX convierte las celdas de fecha a `AAAA-MM-DD`.

## Consecuencias
- Las fases siguientes (novedades y liquidación, desprendibles, provisiones y liquidación del
  contrato, PILA, nómina electrónica, contabilización) leen el catálogo y las versiones vigentes en
  cada fecha sin cambiar este modelo.
- Mientras la plataforma no cargue el catálogo vigente, no se puede contratar (falla con el
  parámetro que falta): es deliberado, para no inventar valores.
- Quedan para después: el centro de costo del contrato (con la contabilización, fase 7), el
  servicio doméstico y los topes de duración de los contratos (por confirmar con el especialista).
- Sin pruebas de `application` con dobles: los casos de uso se prueban contra PostgreSQL
  (`infra-postgres/tests/nomina.rs`) y por la API (`apps/api/tests/nomina_api.rs`), como los
  cobros (plan 19).
