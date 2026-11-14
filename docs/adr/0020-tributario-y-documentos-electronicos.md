# ADR 0020 · Tributario y documentos electrónicos

- **Estado:** aceptada
- **Fecha:** 2026-11-13
- **Plan:** `docs/planes/15-contabilidad-tributario-y-documentos-electronicos.md` (fases 1–5; la 6
  solo diseñada)

## Contexto
Sobre el núcleo contable (ADR 0017), la tesorería (ADR 0018) y la cartera de la PH (ADR 0019),
cualquier entidad necesita: saber si es responsable de IVA, agente de retención o facturador
electrónico; facturar con IVA solo lo que lo causa (una PH no factura expensas); emitir factura
electrónica, notas crédito y documento soporte ante la DIAN; recibir las facturas de sus
proveedores con sus eventos RADIAN; y **preparar** retención en la fuente, IVA, ICA, información
exógena y el RUB. Las tarifas, bases y formatos cambian cada año, y el proveedor tecnológico aún
no está elegido (decisión D3 etapa 1); el motor propio (D3 etapa 2) exige habilitación ante la
DIAN.

## Decisión
- **El sistema prepara; no presenta.** Presentar y firmar declaraciones lo hace el responsable
  tributario. Nada de tarifas en el código: IVA (`tarifas_iva`), ICA (`tarifas_ica`), formatos de
  exógena (`formatos_exogena` + `campos_formato_exogena`, versión por año y código) son catálogos
  de **plataforma** versionados (vigencia + fuente), como la UVT y las retenciones del plan 13.
- **Perfil tributario versionado** (`perfiles_tributarios`): lo crea quien configura
  (`tributario.configurar`) en **borrador** y lo confirma el contador
  (`tributario.perfil.confirmar`); confirmado no se modifica (trigger, `V0002`): cada cambio es
  una versión nueva. **Sin perfil confirmado no hay automatismos**: sin él no hay IVA en ventas,
  no se practican retenciones (antes del plan 15 se practicaban siempre) ni se emiten documentos
  electrónicos. `ParametrosPerfil::validar` exige coherencia con el RUT (O-13, O-15, R-99-PN).
- **IVA solo donde se causa.** Cada línea de venta lleva `clasificacion` (`expensa`,
  `cuota_extraordinaria`, `uso_sin_cobro`, `alquiler_servicio`, `venta`, `otra`); solo
  `alquiler_servicio` y `venta` pueden llevar IVA (`validar_iva_linea` + `CHECK`), con una tarifa
  gravada vigente del catálogo elegida por quien factura. Los conceptos de cobro de la PH guardan
  su `clasificacion_tributaria`.
- **Módulo `facturacion_electronica`** (no exclusivo de la PH). Un documento de negocio produce
  **un solo** documento electrónico por tipo (`UNIQUE (tenant, tipo, origen)`): pedirlo otra vez
  devuelve el mismo documento y el mismo CUFE. La numeración sale de una **resolución activa y
  vigente** (`resoluciones_numeracion`, bloqueada al numerar); número y CUFE únicos. Estados solo
  por `EstadoDocumentoElectronico::transicionar` (un `rechazado` nunca pasa a `validado`, QA-015);
  una caída del proveedor deja el documento en `contingencia` para reintentar. Eventos
  append-only. Con factura electrónica validada, una factura de venta solo se anula con **nota
  crédito electrónica**.
- **Puerto `ProveedorFacturacion`** con un `DocumentoAEmitir` neutro (no UBL): sirve igual para un
  proveedor tecnológico que para el motor propio. Mientras se elige el proveedor, el crate
  `infra-facturacion` trae `ProveedorSimulado` (CUFE determinista SHA-384, XML de muestra sin
  validez); el adaptador real se agrega al lado sin tocar los casos de uso.
- **Facturas recibidas** por XML (Base64): `LectorUbl` (roxmltree, sin DTD ni entidades externas)
  lee la `Invoice` o el `AttachedDocument`; sin duplicados (CUFE y SHA-256); se asocian al
  proveedor por NIT; se vinculan a la factura de proveedor con el mismo CUFE o se rechazan. Los
  eventos RADIAN (`acuse` → `recibo_bien` → `aceptacion`/`reclamo`) exigen sus previos y van por el
  proveedor (exigen el módulo). Leer y cargar el buzón no lo exige: toda entidad recibe facturas.
- **Exógena reproducible**: suma por tercero los movimientos contabilizados de las cuentas de cada
  concepto (sin comprobantes revertidos ni sus reversos), agrega las cuantías menores en el NIT
  222222222 y guarda el CSV con su SHA-256 (`generaciones_exogena`, append-only): la misma versión
  y los mismos datos dan el mismo archivo.
- **RUB** (`beneficiarios_finales`): participación solo con el criterio de propiedad; ser
  propietario de un bien de una PH no hace a nadie beneficiario final de la PH.

## Consecuencias
- Las entidades que ya practicaban retenciones deben crear y confirmar su perfil tributario.
- El núcleo tributario es genérico (pruebas de fronteras); la vertical PH solo aporta la
  clasificación de sus conceptos.
- **Fase 6 (motor propio) diseñada, no construida**: depende de D3 etapa 2 y de la habilitación
  ante la DIAN; entraría como otro adaptador de `ProveedorFacturacion`.
- **Diferido**: la plataforma como entidad que factura sus propias suscripciones y la publicidad.
  Usará este mismo flujo cuando exista su entidad contable (sin cambios de diseño).
- La representación gráfica es un PDF con el CUFE; el QR de la DIAN llega con el proveedor real.
