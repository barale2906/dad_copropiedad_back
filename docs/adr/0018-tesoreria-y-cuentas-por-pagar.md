# ADR 0018 · Tesorería, cuentas por pagar, facturas de venta y conciliación

- **Estado:** aceptada
- **Fecha:** 2026-11-13
- **Plan:** `docs/planes/13-contabilidad-tesoreria-y-cuentas-por-pagar.md` (fases 1–4)

## Contexto
Sobre el núcleo contable (ADR 0017) cualquier entidad necesita manejar su dinero: cuentas
bancarias y cajas, recaudos, pagos a proveedores con retenciones, facturas de venta (la PH las
usa para el uso de zonas comunes) y la conciliación con el banco. Todo debe servir a cualquier
vertical sin conocer la PH, reutilizar lo existente (personas como terceros, consecutivos,
archivos, auditoría, outbox, `infra-pdf`, `LectorPlanillas`) y no escribir asientos fuera del
contabilizador.

## Decisión
- **Documentos y eventos.** Cada documento se entrega al contabilizador con un evento y conceptos
  fijos (`casos_uso::contabilidad::tesoreria::{eventos, conceptos}`): `tesoreria.recibo`
  (`recibido`, `aplicado`, `sin_aplicar`), `tesoreria.aplicacion`, `tesoreria.egreso`
  (`pagado_facturas`, `gasto`, `girado`), `cxp.factura` (`gasto`, `iva_descontable`,
  `retencion_renta|iva|ica`, `por_pagar`) y `ventas.factura` (`por_cobrar`, `ingreso`,
  `iva_generado`). El contador define la regla de cada evento (P1).
- **Cuenta del documento.** `ImporteDocumento` lleva `cuenta` (opcional) y `base`: la cuenta de
  gasto o ingreso de cada línea y la del banco del recibo o egreso reemplazan la cuenta por
  defecto de la regla, **solo** en conceptos de una línea. La retención usa la cuenta sugerida
  del catálogo si la entidad la tiene; si no, la de la regla. La base queda en la línea del
  asiento (certificados y medios magnéticos).
- **Anular = revertir.** Recibos, egresos y facturas se anulan revirtiendo su comprobante
  (`revertir_en`, dentro de la misma transacción) y devolviendo saldos; nada se borra salvo una
  factura de venta en borrador.
- **Un solo mecanismo de recaudo.** Los recibos de caja se aplican a facturas de venta con
  `aplicaciones_pago` (el plan 14 agrega las obligaciones de la PH a la misma tabla). Lo no
  aplicado queda **sin aplicar** (anticipo o recaudo por identificar) y se aplica después con su
  propio comprobante; un recibo sin pagador queda identificado al aplicarlo.
- **Segregación de funciones.** Un egreso se registra, lo autoriza **otra cuenta** (también un
  `CHECK` en la BD) y luego se paga (ahí se contabiliza). La excepción de una factura de
  proveedor duplicada exige `tesoreria.egresos.autorizar`, motivo y queda auditada; el índice
  único parcial la hace cumplir.
- **Retenciones** (plan 15 fase 1, adelantado): catálogo de plataforma versionado por vigencia
  (`conceptos_retencion`, `uvt`; `plataforma.normativa.gestionar`). Se calculan al registrar la
  factura con la versión vigente en su fecha: base mínima en UVT del año, redondeo al peso, sin
  retención de renta a autorretenedores (O-15) ni al régimen simple (O-47), sin ReteIVA a quien
  no es responsable de IVA. La PH no siembra tarifas reales: las carga la plataforma.
- **Facturas de venta** genéricas: IVA por línea elegido por quien factura (nunca automático) y
  número del comprobante `FV` mientras llega la numeración DIAN (plan 15).
- **Extractos.** La lectura es lógica pura de dominio (`domain::contabilidad::extracto`): planilla
  CSV/XLSX con mapeo de columnas por banco (leída con el `LectorPlanillas` existente, movido a
  `puertos::planillas`) y recaudo Asobancaria 2001; cada lector tiene versión. El archivo queda en
  `archivos`; el SHA-256 impide importarlo dos veces en la misma cuenta. No se creó un puerto
  `LectorExtractos`: el adaptador solo entrega filas de texto.
- **Conciliación** append-only: un registro agrupa movimientos y líneas de libros cuyas sumas
  coinciden (uno a uno, varios a uno o varios a varios); deshacer es otro registro que apunta al
  original. La automática casa por valor exacto, fecha con tolerancia (≤ 10 días) y referencia.
  Un crédito sin recibo genera un **recibo no identificado** conciliado en la misma transacción.
  El informe muestra saldo del extracto, de libros, partidas conciliatorias y la diferencia.
- **Pasarela** (fase 4): `eventos_pasarela` append-only, único por (proveedor, id del evento);
  `aprobado` crea el recibo (uno por referencia), `reversado`/`contracargo` lo anulan. El
  adaptador de la pasarela, su webhook en el `gateway` y el puerto para crear intenciones de pago
  llegan cuando se elija la pasarela (decisión abierta D8).
- **Montos** con exactamente dos decimales (`Monto`): la API siempre responde `"1500.00"`.

## Consecuencias
- Cualquier vertical cobra y paga con estas piezas; la PH (plan 14) solo agrega sus obligaciones
  y su cartera por bien.
- Siete permisos nuevos `tesoreria.*`; el contador recibe todos menos autorizar egresos (lo tiene
  el presidente del consejo y el administrador).
- Pendiente: adaptador de pasarela y webhook, nota crédito y numeración electrónica (plan 15),
  PDF de egresos y recibos, comisiones de la pasarela en la liquidación, certificados con el PDF
  adjunto al correo (hoy el correo lleva el detalle en el texto), cartera de proveedores por
  edades y rutas de roles para entidades sin vertical.

## Adenda (2026-11-27): soportes en PDF y cuentas por pagar por edades
De los pendientes de arriba quedan resueltos:
- **PDF de recibos y egresos** (`casos_uso::contabilidad::soportes`): se generan al vuelo con la
  tabla de `infra-pdf`; muestran el número del comprobante, a qué se aplicó el recibo (y el
  anticipo) o qué facturas y gastos cubre el egreso, y marcan la anulación. El egreso lleva
  espacio para la firma del beneficiario.
- **Certificados con el PDF adjunto**: el certificado de retenciones (y las cuentas de cobro de
  la cartera de la PH, ADR 0019) viajan por el outbox con su PDF como `AdjuntoCorreo`.
- **Cuentas por pagar por edades** (`cuentas_por_pagar::por_pagar_por_edades`): neto a pagar −
  pagado de las facturas registradas, causadas o pagadas en parte, por tramo de días vencidos;
  usa el mismo agrupador que la cartera de clientes (`cuentas_por_cobrar::agrupar_por_edades`).

Siguen pendientes: el adaptador de la pasarela y su webhook (espera la elección de la pasarela),
las comisiones de la pasarela en la liquidación y las rutas de roles para entidades sin vertical.
