# ADR 0022 · Facturación electrónica como módulo independiente

- **Estado:** aceptada.
- **Fecha:** 2026-11-14
- **Plan:** `docs/planes/18-facturacion-electronica-independiente.md` (parte A)

## Contexto
En el mercado colombiano la facturación electrónica se compra **sola**: muchas empresas llevan la
contabilidad en otro programa o con un contador externo y solo necesitan facturar. El módulo
`facturacion_electronica` (ADR 0020) dependía de hecho de `contabilidad`, sin declararlo: todos sus
casos de uso abrían con la transacción de tesorería, que exige la contabilidad; la línea de una
factura de venta exigía una cuenta del PUC; emitir siempre contabilizaba y el número de la factura
era el del comprobante `FV`. Habilitar la facturación sola daba `modulo_no_habilitado` en todo.
Además, la tesorería (contabilidad) importaba el código de las ventas para aplicar los recaudos.

## Decisión
- **La facturación se vende sola y la contabilidad no la conoce.** `casos_uso::facturacion`
  (`items`, `ventas`, `documentos`, `contabilizacion`, `exportaciones`) puede usar el núcleo
  contable; la contabilidad no importa la facturación (prueba
  `la_contabilidad_no_importa_la_facturacion`). Lo tributario compartido pasa a `domain::tributario`
  y `casos_uso::tributario`. El recaudo y la cartera de clientes son de la contabilidad
  (`contabilidad::cuentas_por_cobrar`).
- **Módulos:** las ventas, el perfil tributario, las resoluciones y los terceros sirven con
  `facturacion_electronica` **o** `contabilidad` (`modulos::abrir_con_modulos`); emitir
  documentos electrónicos exige `facturacion_electronica`; contabilizar lo pendiente exige
  `contabilidad`.
- **Sin contabilidad:** la línea no lleva cuenta ni centro de costo (`cuenta_id` opcional), la
  factura emitida no tiene comprobante, el pago se **marca a mano** (fecha, medio, referencia y
  quién; el `CHECK` lo admite solo en una factura pagada y sin contabilizar), y la compra para el
  documento soporte se registra sin cuentas ni retenciones (y el documento soporte sale de la
  compra registrada). **Con contabilidad** todo se contabiliza en la misma transacción, como antes;
  una línea sin cuenta usa la de la regla del evento.
- **Número interno** de la factura por año en `consecutivos` (serie `factura_venta`); el número
  legal es el del documento electrónico de la DIAN.
- **Catálogo de productos y servicios** (`items_facturacion`, por tenant, con unidad DIAN, precio
  sugerido, IVA, clasificación y cuenta opcional): una línea puede salir de un ítem y tomar de él
  lo que no traiga.
- **Contabilizar después:** quien habilita la contabilidad más tarde contabiliza lo facturado con
  `contabilizar_pendientes` (una transacción por factura, idempotente; informe de las pagadas a
  mano y de las fallidas).
- **Para el contador externo:** libros de ventas y compras en CSV y paquete ZIP con el XML y el PDF
  de los documentos del periodo (puerto `Empaquetador`, implementado con el ZIP del motor propio),
  auditados con su huella SHA-256 (permiso `facturacion.exportar`).
- **Consumo por entidad** para cobrar el módulo: `consumo_documentos_electronicos` (SECURITY
  DEFINER, solo conteos) en `GET /plataforma/facturacion/consumo`.
- **Permisos:** `facturacion.ver`, `facturacion.facturas.gestionar` (antes
  `tesoreria.facturas_venta.gestionar`), `facturacion.items.gestionar`, `facturacion.exportar` y
  `terceros.gestionar` (antes `contabilidad.terceros.gestionar`); plantilla de rol `facturador`.
- **Rutas** (sin periodo de transición: no había frontend):
  - `/entidades/{id}/facturacion/…`: `items`, `facturas` (con `emision`, `pago`, `anulacion`,
    `factura-electronica`, `nota-credito`), `compras` (y `…/documento-soporte`), `documentos`,
    `recibidas`, `libro-ventas`, `libro-compras`, `paquete`, `contabilizacion`.
  - Comunes a toda entidad en `/entidades/{id}/…`: `terceros`, `tributario/{catalogo,perfiles,
    resoluciones}`, `roles`, `asignaciones`, `cuentas`, `archivos` y `auditoria` (antes bajo
    `/copropiedades/{id}`; en una PH el id es el mismo).
  - En la contabilidad quedan las declaraciones (`/contabilidad/tributario/…`) y la cartera de
    clientes (`/contabilidad/cuentas-por-cobrar/edades`).
- Migración `20261114000330_facturacion_independiente`.

## Consecuencias
- Una entidad puede contratar contabilidad, facturación o las dos; la nómina (plan 16) seguirá la
  misma regla.
- Las facturas pagadas a mano que luego se contabilizan dejan la cuenta por cobrar abierta: el
  informe las señala para que el contador registre su recaudo.
- Pendiente: la nota débito electrónica (el tipo existe en el esquema; falta decidir cómo se
  recauda) y la habilitación ante la DIAN por entidad (plan 18, parte B).
