-- Reversa de la facturación independiente. Falla a propósito si ya hay facturas o compras sin
-- cuenta contable o emitidas sin comprobante: esos datos no caben en el esquema anterior.

DROP FUNCTION consumo_documentos_electronicos(date, date);
DELETE FROM roles_permisos WHERE permiso = 'facturacion.exportar';
DELETE FROM permisos WHERE codigo = 'facturacion.exportar';
DELETE FROM roles_permisos WHERE rol_id = uuid_rol_plantilla('facturador');
DELETE FROM roles WHERE id = uuid_rol_plantilla('facturador');
DELETE FROM roles_permisos WHERE permiso IN ('facturacion.ver', 'facturacion.items.gestionar');

INSERT INTO permisos (codigo, ambito) VALUES
    ('tesoreria.facturas_venta.gestionar', 'copropiedad'),
    ('contabilidad.terceros.gestionar', 'copropiedad');
UPDATE roles_permisos SET permiso = 'tesoreria.facturas_venta.gestionar'
WHERE permiso = 'facturacion.facturas.gestionar';
UPDATE roles_permisos SET permiso = 'contabilidad.terceros.gestionar'
WHERE permiso = 'terceros.gestionar';
DELETE FROM permisos WHERE codigo IN ('facturacion.ver', 'facturacion.facturas.gestionar',
                                      'facturacion.items.gestionar', 'terceros.gestionar');

DROP INDEX ix_facturas_proveedor_sin_contabilizar;
ALTER TABLE facturas_proveedor ADD CONSTRAINT facturas_proveedor_check5
    CHECK (estado IN ('registrada', 'anulada') OR comprobante_id IS NOT NULL);
ALTER TABLE lineas_factura_proveedor ALTER COLUMN cuenta_id SET NOT NULL;

DELETE FROM consecutivos WHERE serie = 'factura_venta';
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo') OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

DROP INDEX ix_facturas_venta_sin_contabilizar;
ALTER TABLE facturas_venta
    DROP CONSTRAINT ck_pago_manual_pagada,
    DROP CONSTRAINT ck_pago_manual_completo,
    DROP COLUMN pago_manual_por,
    DROP COLUMN pago_manual_referencia,
    DROP COLUMN pago_manual_medio,
    DROP COLUMN pago_manual_fecha;
ALTER TABLE facturas_venta DROP CONSTRAINT ck_factura_venta_emitida;
ALTER TABLE facturas_venta ADD CONSTRAINT facturas_venta_check5
    CHECK (estado IN ('borrador', 'anulada')
           OR (comprobante_id IS NOT NULL AND numero IS NOT NULL AND emitida_en IS NOT NULL));

DROP INDEX ix_lineas_venta_item;
ALTER TABLE lineas_factura_venta
    DROP CONSTRAINT fk_linea_venta_item,
    DROP COLUMN item_id,
    ALTER COLUMN cuenta_id SET NOT NULL;

DROP TABLE items_facturacion;
