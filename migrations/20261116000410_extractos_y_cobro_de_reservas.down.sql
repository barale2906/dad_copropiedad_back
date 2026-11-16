DROP INDEX ux_cargo_valor_reserva;
DELETE FROM cargos_pendientes WHERE concepto = 'valor_reserva' AND estado = 'pendiente';
ALTER TABLE cargos_pendientes DROP CONSTRAINT cargos_pendientes_concepto_check;
ALTER TABLE cargos_pendientes ADD CONSTRAINT cargos_pendientes_concepto_check
    CHECK (concepto IN ('danos', 'penalidad_cancelacion')) NOT VALID;

ALTER TABLE recursos
    DROP CONSTRAINT fk_recursos_item_facturacion,
    DROP CONSTRAINT ck_recursos_item_facturacion,
    DROP COLUMN item_facturacion_id,
    DROP COLUMN destino_cobro;

ALTER TABLE cuentas_tesoreria DROP COLUMN mapeo_extracto;
