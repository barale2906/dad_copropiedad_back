-- Revierte la tesorería, las cuentas por pagar, las facturas de venta y la conciliación.

DELETE FROM roles_permisos WHERE permiso LIKE 'tesoreria.%';
DELETE FROM permisos WHERE codigo LIKE 'tesoreria.%';

-- Los triggers de solo inserción impiden UPDATE, DELETE y TRUNCATE, no DROP.
DROP TABLE eventos_pasarela;
DROP TABLE conciliacion_partidas;
DROP TABLE conciliaciones;
DROP TABLE movimientos_extracto;
DROP TABLE extractos_bancarios;
DROP TABLE aplicaciones_pago;
DROP TABLE lineas_factura_venta;
DROP TABLE facturas_venta;
DROP TABLE egreso_facturas;
DROP TABLE retenciones_factura_proveedor;
DROP TABLE lineas_factura_proveedor;
DROP TABLE facturas_proveedor;
DROP TABLE egreso_gastos;
DROP TABLE egresos;
DROP TABLE recibos_caja;
DROP TABLE cuentas_tesoreria;
DROP TABLE conceptos_retencion;
DROP TABLE uvt;
