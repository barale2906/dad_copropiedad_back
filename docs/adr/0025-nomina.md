# ADR 0025 · Nómina que se vende sola: catálogo laboral de plataforma, empleados y contratos versionados

- **Estado:** aceptada (fases 1 a 4; de las fases 5 y 7, lo que no depende de terceros)
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

## Decisión de la fase 2: novedades y liquidación
- **Catálogo**: parámetros nuevos (tope del IBC, tope de la exoneración, renta exenta laboral y
  sus topes en UVT anuales, límite de beneficios) y **tablas por tramos** versionadas
  (`tramos_laborales`: retención por salarios en UVT con tarifa marginal y UVT adicionales, y
  fondo de solidaridad en salarios mínimos). Una versión = todos los tramos con el mismo
  `vigente_desde`; contiguos desde cero y con el último abierto (`TablaTramos`). La UVT sale de la
  tabla `uvt` del catálogo tributario (no se duplica).
- **Configuración por entidad** (`configuraciones_nomina`): la exoneración del art. 114-1 del
  ET la decide la entidad (`nomina.configurar`), porque depende de su situación tributaria (una PH
  no suele ser contribuyente).
- **Novedades** de solo inserción (`novedades_nomina`), con su medida (horas en una fecha, un
  rango de días o un valor); se anulan con un registro aparte con motivo. Las dominicales se
  validan contra el domingo o el festivo de `domain::calendario`. Las incapacidades y licencias
  de maternidad o paternidad son `dato_sensible` (sin diagnóstico): solo con
  `personas.datos_sensibles.ver`; los valores en pesos, solo con `nomina.salarios.ver`.
- **Periodos** `abierto → liquidado → aprobado → pagado` (`EstadoPeriodoNomina::transicionar`):
  mes completo o quincenas 1–15 y 16–fin. Liquidar se repite mientras no esté aprobado
  (reemplaza todo). **Aprueba otra cuenta** (también `CHECK`), y no si hubo novedades después de
  liquidar. Aprobado, la BD rechaza cambiar sus liquidaciones o registrar y anular novedades en
  sus fechas. La corrección será la nómina de ajuste (fase 6).
- **Cálculo puro** (`domain::nomina::liquidacion::liquidar`, por contrato y periodo) con las
  convenciones del encabezado del módulo, **para revisión del especialista**: días comerciales de
  30, hora = salario ÷ (jornada ÷ 6 × 30), incapacidad común al porcentaje del catálogo sin bajar
  del mínimo diario, auxilio de transporte con salario ordinario hasta el tope (inclusive) por días
  laborados, IBC con la regla del 40 % y entre un mínimo proporcional y el tope, aportes con
  exoneración, aprendiz solo salud a cargo del empleador y ARL en etapa productiva, retención por
  el procedimiento 1 (renta exenta del 25 % con sus topes, aproximada a mil) sobre el mes: en la
  nómina quincenal la segunda quincena retiene con lo liquidado en la primera.
- **Permisos**: `nomina.configurar`, `nomina.novedades.registrar`, `nomina.liquidar`,
  `nomina.aprobar`. Ver una liquidación exige `nomina.salarios.ver`.
- Pendiente para después: vacaciones y prestaciones (fase 4), deducciones de la retención
  (dependientes, vivienda, medicina prepagada) y el procedimiento 2, cotización por semanas de
  la jornada parcial y la nómina de ajuste.

## Decisión de la fase 3: desprendibles y portal del empleado
- El desprendible es el **PDF tabular** que ya genera `infra-pdf` (`TablaPdf`): no hay plantilla
  nueva. Solo existe con el periodo **aprobado** o pagado; no lleva la cuenta bancaria ni los
  aportes del empleador, y las incapacidades y licencias salen como «Ausencia remunerada»
  (`ConceptoNomina::etiqueta_desprendible`): el desprendible circula y el motivo es sensible.
- Envío por correo con el PDF adjunto **por el outbox** (sale solo si la transacción confirma);
  quien no tiene correo se cuenta en `sin_correo`.
- **Portal del empleado** en `/yo/nomina`: el empleado no necesita roles en la entidad. El listado
  cruza entidades solo por la función `SECURITY DEFINER` `desprendibles_de_cuenta` (lo mínimo:
  entidad, periodo y totales); la descarga abre la transacción de la entidad sin permisos y
  verifica que la persona de la liquidación esté vinculada a la cuenta (404 si no, sin distinguir
  si existe).

## Decisión de la fase 4: prestaciones sociales y liquidación del contrato
- **Provisiones** en cada liquidación (clase `provision`, no tocan el neto): cesantías y prima
  sobre lo salarial más el auxilio, intereses (12 % anual) sobre la provisión de cesantías y
  vacaciones sobre el salario ordinario. El integral solo provisiona vacaciones y el aprendiz,
  nada.
- **Novedad `vacaciones`**: los días calendario del descanso, pagados al salario diario (salarial).
  El saldo cuenta los días **hábiles** disfrutados con `domain::calendario::es_dia_habil` (lunes a
  viernes no festivos: por confirmar si el sábado es hábil en cada entidad).
- **Pagos dentro del periodo** (`casos_uso::nomina::prestaciones`), sin documentos aparte: así se
  aprueban, se reflejan en el desprendible y llegarán a la PILA y a la nómina electrónica con el
  resto. Prima en junio (periodo que termina el 30) y diciembre (periodo que incluye el 20);
  intereses del año anterior en el primer periodo de enero; al **terminar el contrato** en el
  periodo: cesantías e intereses del año, prima del semestre, vacaciones pendientes en dinero e
  indemnización (CST art. 64, `domain::nomina::prestaciones`).
- **Base** de cada lapso: último salario, o el promedio si cambió en los últimos tres meses, más
  el auxilio (no en vacaciones) y el promedio de lo variable ya liquidado en otros periodos del
  lapso. Las prestaciones no son IBC ni entran a la regla del 40 %; cesantías, intereses e
  indemnización no entran a la retención del procedimiento 1 (exentas o con retención propia:
  **por revisar** con el especialista, también el tope de 350 UVT del art. 206 num. 4).
- **Cesantías por consignar** (`GET …/cesantias?anio=`): por contrato con salario ordinario
  vigente al 31 de diciembre, con su fondo; consignarlas es del empleador en su banco.

## Decisión de las fases 5 y 7 (lo que no depende de terceros)
- **Contabilización** (`casos_uso::nomina::contabilizacion`), solo con `Modulo::Contabilidad`: la
  nómina entrega documentos origen al contabilizador en la misma transacción. Al **aprobar**,
  `nomina.liquidacion` por liquidación (tercero: la persona del empleado) con un concepto por cada
  `ConceptoNomina` (su texto) y `neto`; al **pagar**, `nomina.pago` con `neto` y `girado`. Las
  cuentas las pone el contador en la regla de cada evento (los aportes y provisiones llevan
  débito y crédito en la misma regla). Sin regla vigente, aprobar falla con el nombre del evento.
- **Aportes del mes** (`GET …/aportes?anio=&mes=`): por empleado y por administradora, con el tipo
  de cotizante, más ICBF y SENA: el insumo para la PILA mientras no se confirme el archivo plano.
- **Ingresos y retenciones del año** (`GET …/ingresos-retenciones?anio=`): totales por grupo del
  certificado (`GrupoCertificado`); el formulario 220 tiene formato prescrito y se genera cuando se
  tenga su especificación vigente.
- Pendientes de terceros: archivo plano de la PILA (operador), nómina electrónica (anexo técnico
  y registro en la DIAN), dispersión por banco (formato de cada banco) y formulario 220.

## Complementos (2026-11-20)
- **Centro de costo** opcional en cada versión del contrato (solo con contabilidad; FK a
  `centros_costo`): va en los importes de `nomina.liquidacion` y la regla lo usa en las líneas con
  centro `documento`.
- **Contabilizar lo aprobado antes de tener contabilidad** (`POST …/contabilizacion`): cada periodo
  aprobado o pagado en su transacción, idempotente por origen, con informe de fallidos.
- **Simulación de la terminación** (sin guardar nada) y **saldo de vacaciones** por contrato, con
  el mismo cálculo de la liquidación real.
- **Importación de novedades** (tipo `novedades_nomina`): empleado por código y contrato que cubre
  las fechas; se aplica en una transacción.
- **Deducciones de la retención** declaradas por el empleado (`deducciones_retencion`: dependientes,
  intereses de vivienda, medicina prepagada y aportes voluntarios), con topes en el catálogo
  (`porcentaje_dependientes`, `tope_*` en UVT anuales, `limite_aportes_voluntarios`). Depuración:
  ingresos − aportes obligatorios − deducciones − aportes voluntarios → renta exenta del 25 % → todo
  limitado al 40 % y a 1340 UVT anuales. **Por revisar** con el especialista: los 72 UVT por
  dependiente de la Ley 2277 de 2022 y el procedimiento 2 no están.
- **Certificado laboral** en PDF (`TablaPdf`): cargo, tipo de contrato y fechas, y el salario actual
  solo si se pide con `nomina.salarios.ver`; el empleado descarga el suyo (con salario) en
  `/yo/nomina/{entidad_id}/certificado-laboral` sin roles en la entidad.
- **Avisos automáticos** (worker `worker_nomina` del gateway, cada 6 horas): a las cuentas con
  `nomina.empleados.gestionar`, por correo y una vez por referencia (`avisos_nomina`): contratos a
  término fijo que vencen en 45 días (el preaviso es de 30: CST art. 46) y recordatorios de los
  intereses de cesantías (31 de enero), su consignación (14 de febrero) y la prima (30 de junio y
  20 de diciembre). Las entidades se recorren con la función `SECURITY DEFINER`
  `entidades_con_nomina`.

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
