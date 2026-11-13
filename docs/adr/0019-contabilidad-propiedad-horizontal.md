# ADR 0019 · Vertical contable de la propiedad horizontal (cartera por bien privado)

- **Estado:** aceptada
- **Fecha:** 2026-11-14
- **Plan:** `docs/planes/14-contabilidad-vertical-ph.md` (fases 1–7)

## Contexto
Sobre el núcleo contable (ADR 0017) y la tesorería (ADR 0018), una copropiedad necesita liquidar
sus expensas (Ley 675 arts. 29, 30, 35, 48, 51), cobrar con cuentas de cobro (las expensas no se
facturan), llevar la cartera **por bien privado** con imputación de pagos (Código Civil art.
1653), pronto pago, anticipos y cartera inicial, y cuadrar el sub-libro contra el mayor (CTCP).
Todo debe montarse sobre lo que la PH ya tiene (bienes privados, cuadros de coeficientes,
titulares, preguntas de la asamblea, cargos de las reservas) y sobre los recibos y aplicaciones
de pago de la tesorería, sin maestros paralelos ni asientos fuera del contabilizador.

## Decisión
- **Módulo `contabilidad_ph`** (`Modulo::ContabilidadPh`, solo en una copropiedad). Sus casos de
  uso exigen además `contabilidad`. Código propio en `domain::contabilidad_ph`,
  `casos_uso::contabilidad_ph` y `puertos::cartera`: usan el núcleo; el núcleo no los conoce
  (las pruebas de fronteras siguen igual).
- **Eventos y conceptos.** Cada documento entra al contabilizador con un evento `ph.*`
  (`ph.liquidacion`, `ph.cobro`, `ph.recaudo`, `ph.aplicacion`, `ph.nota_credito`,
  `ph.pronto_pago`); los conceptos de cada regla son el `concepto_contable` de los conceptos de
  cobro, que elige el contador (P1). El deudor en el mayor es el **propietario representante**
  del bien (decisión del usuario); el pagador (tenedor, art. 29) solo recibe la cuenta de cobro y
  firma el recibo.
- **Conceptos de cobro** con tipo (`ordinaria`, `fondo_imprevistos`, `extraordinaria`,
  `uso_bien_comun`, `multa`, `interes_mora`, `dano`, `otro`) y criterio de reparto
  (`coeficiente`, `modulo`, `valor_fijo`, `por_tipo_bien` con `tarifas_concepto_cobro`, `area`).
  **Política de cartera** versionada y de solo inserción (vencimiento, redondeo, pronto pago,
  factor de mora ≤ 1,5, imputación, fondo ≥ 1 % salvo suspensión; pregunta de la asamblea).
- **Módulos de contribución sin tablas nuevas de coeficientes:** `cuadros_coeficientes` ganó
  `modulo_id`; el índice de "un vigente" y el de versión pasaron a `(tenant, tipo, módulo)`. El
  cuadro de un módulo usa el mismo flujo (crear, cargar, revisar, activar), suma 1 dentro del
  módulo y solo lleva sus bienes; no exige todos los bienes activos ni bloquea por asamblea.
- **Presupuesto** por versiones (`borrador` → `vigente` → `reemplazado`) con rubros mensuales por
  concepto y cuenta (inmutables una vez aprobado, trigger). La ejecución presupuestal es un
  reporte contra el mayor (sin tabla).
- **Liquidación** en dominio puro (`liquidar`): lo ordinario se reparte con **el método del mayor
  residuo** (`repartir`), así que cada concepto cuadra al peso; el fondo es el porcentaje de lo
  ordinario de cada bien en línea aparte; las cuotas extraordinarias siguen su calendario. Usa el
  presupuesto vigente del año, la política vigente en el periodo y el **cuadro vigente al cierre
  del periodo** (QA-008). La "liquidación en borrador" es la **vista previa** (no se guarda); al
  liquidar se guardan la liquidación (una vigente por periodo), las obligaciones (índice único
  por bien, concepto, periodo y cuota: QA-005), un comprobante con una línea por concepto y
  deudor, y una **cuenta de cobro** inmutable por bien (consecutivo `CB` por año en
  `consecutivos`). Los cargos pendientes de las reservas entran en la liquidación como
  obligaciones (`origen = cargo`). Anular una liquidación sin pagos revierte su comprobante;
  con pagos se corrige con notas crédito.
- **Recaudo = recibo de caja de la tesorería** con aplicaciones a obligaciones:
  `aplicaciones_pago` ganó `obligacion_id` (exactamente un destino). El núcleo solo guarda la
  aplicación (`AplicacionesExternas` en `registrar_recibo_con`); el **saldo se reconstruye**
  (valor − ajustes − aplicaciones vigentes, QA-012). La imputación es dominio (intereses antes que
  capital; el pagador elige solo si la política lo consiente). Lo que sobra queda sin aplicar en
  el recibo (**anticipo**, sin tabla nueva) y la siguiente liquidación lo aplica si su dueño paga
  un solo bien. El **pronto pago** es un ajuste con su comprobante cuando el pago llega a tiempo y
  cubre la deuda menos el descuento; anular el recaudo lo revierte. El núcleo rechaza anular desde
  la tesorería un recibo que pagó obligaciones: se anula desde la cartera.
- **Recaudo bancario por referencia:** `bienes_privados.referencia_pago` (numérica, única); los
  créditos pendientes del extracto (planilla o Asobancaria) con esa referencia se vuelven recaudos
  conciliados en la misma transacción (`recibo_de_movimiento_en`).
- **Ajustes** (`ajustes_obligacion`): nota crédito o pronto pago, cada uno con su comprobante;
  se anulan marcando `anulado_en` y revirtiendo el comprobante.
- **Cartera inicial** por JSON con vista previa (`solo_validar`): obligaciones `saldo_inicial`
  sin comprobante (su contabilidad son los saldos iniciales del núcleo); con errores no carga
  nada. El **cuadre** compara, por cada cuenta que debita la regla de `ph.liquidacion`, el saldo
  del mayor con la suma de los saldos de las obligaciones.
- **Mora (fase 5).** La plataforma carga el interés bancario corriente certificado
  (`tasas_referencia`, sin solapes por modalidad, solo inserción). La tasa de mora es el factor de
  la política (≤ 1,5) por esa tasa o la tasa inferior aprobada (convertida a efectiva anual), y el
  interés de cada día es el equivalente de la efectiva anual sobre el **capital** pendiente ese
  día (pagos y ajustes por fecha), por tramos de tasa y de saldo (QA-010). Cada causación
  (`causaciones_mora`, una vigente por periodo, evento `ph.intereses`) crea una obligación de
  origen `interes` por bien con el concepto de tipo `interes_mora`, que nunca genera intereses
  (sin anatocismo), y deja el detalle de cada tramo en `causaciones_interes` (base, días, tasa y
  la tasa de referencia). Anular una causación sin pagos revierte su comprobante y deja los días
  por causar otra vez. El pago cubre primero los intereses (imputación).
- **Acuerdos de pago** (`acuerdos_pago` + `cuotas_acuerdo`): cuotas mensuales que cuadran al
  peso; los aprueba otra cuenta (`CHECK`); un acuerdo activo por bien. Mientras está vigente
  suspende la mora de lo que vencía hasta su fecha; al causar se revisa: si una cuota venció sin
  cubrirse queda incumplido y la mora corre desde el día siguiente (no se cobra lo suspendido).
  La deuda original no se borra.
- **Cartera por edades** con los tramos del núcleo (`TramoEdad`).
- **Certificados** (`certificados_emitidos`): paz y salvo solo sin saldo de lo ya cobrado (también
  lo liquidado por anticipado) y certificado de deuda; el PDF lleva un código de 80 bits del que
  solo se guarda el SHA-256, y se verifica sin sesión en
  `POST /copropiedades/{id}/certificados/verificacion` (límite por IP; responde solo válido,
  clase y corte). El PDF y el código se entregan una sola vez.
- **Portal del propietario** (`/copropiedades/{id}/mi-cartera/…`): la cuenta que es titular
  vigente o pagador de un bien ve su estado de cuenta y sus cuentas de cobro (con PDF) y descarga
  su paz y salvo, sin permisos de cartera; el bien de otro responde 404.
- **Integraciones** (las dos son PH, así que la cartera puede usarlas): los morosos de una
  reunión salen de la cartera vencida al corte con el caso de uso de asambleas (que sigue sin
  conocer la contabilidad); la restricción de reservas por mora se sincroniza con un umbral de
  vencido usando `restricciones_reserva` (solo toca las que creó la cartera).
- **Informes** (fase 7): estado del fondo de imprevistos frente al presupuesto, flujo de caja
  mensual de las cuentas de tesorería según el mayor, proyecto de presupuesto del año siguiente
  (copia del vigente con ajuste, en borrador) y paquete PDF para la asamblea (ejecución, edades y
  fondo).
- **Canales** (decisión del usuario): el menú del chat gana «Mi saldo» (solo propietario o
  pagador; puerto mínimo `ResumenCartera` para no cargar al chat con toda la cartera) y «Cita con
  administración» pasa a ser una opción de «Contactar administración» (WhatsApp admite 10 filas).
  Solo los **avisos** son automáticos: el worker del gateway envía el recordatorio de pago 3 días
  antes del vencimiento y el aviso de mora 5 días después, una vez por cuenta de cobro y clase
  (`avisos_cartera`), por correo (con el valor) y por WhatsApp con la plantilla
  `recordatorio_pago` (sin el valor). La causación de intereses y la restricción de reservas siguen
  siendo manuales porque crean asientos y bloqueos.
- Seis permisos nuevos: `cartera.ver`, `cartera.configurar`, `cartera.liquidar`,
  `cartera.ajustar`, `cartera.certificados.emitir`, `presupuesto.gestionar` (contador y
  administrador todos; revisor fiscal, consejero y presidente del consejo `cartera.ver`).

## Consecuencias
- Rutas en `/api/v1/copropiedades/{copropiedad_id}/cartera/…` (etiqueta `cartera`); el PDF de la
  cuenta de cobro se genera al vuelo con `GeneradorActas::tabla` (la cuenta es inmutable).
- Desviaciones del plan: la plantilla estándar PH no trae conceptos ni políticas (el núcleo de
  plantillas no conoce la PH; se crean por la API); la cuenta de cobro no guarda el PDF en
  `archivos`; la cartera inicial es JSON (no el flujo de `importaciones`); el envío masivo es por
  correo (la plantilla de WhatsApp `cuenta_cobro` está por aprobar en Meta y el push necesita
  la cuenta del pagador).
- La causación de intereses se dispara por la API (mensual); falta la tarea programada del
  worker del gateway.
- El umbral de mora de las reservas se pasa al sincronizar (no está en la política de reserva) y
  la sincronización, los morosos y la causación de intereses se disparan por la API.
- Los avisos no van por push todavía (correo y WhatsApp); el paz y salvo por el chat remite a la
  app.
- Pendiente: aprobar en Meta la plantilla `recordatorio_pago`, cobros de zonas comunes
  facturables (plan 15) y pago por PSE (D8).
