-- Plan 20, fase 4.
--
-- S7: la cuenta de tesorería recuerda el último mapeo de columnas de sus extractos (cada banco
--     tiene su planilla; no se configura cada mes).
-- S9: el contador decide cómo se cobra el valor de las reservas de cada recurso: a mano (como
--     hasta ahora), en la cartera (cargo pendiente que sale en la cuenta de cobro, concepto
--     `uso_bien_comun`) o con una factura de venta en borrador con el ítem del recurso.

ALTER TABLE cuentas_tesoreria ADD COLUMN mapeo_extracto JSONB;
COMMENT ON COLUMN cuentas_tesoreria.mapeo_extracto IS
    'Último mapeo de columnas usado al importar un extracto en planilla (domain::MapeoColumnas)';

ALTER TABLE recursos
    ADD COLUMN destino_cobro TEXT NOT NULL DEFAULT 'manual'
        CHECK (destino_cobro IN ('manual', 'cartera', 'factura')),
    ADD COLUMN item_facturacion_id UUID,
    ADD CONSTRAINT ck_recursos_item_facturacion
        CHECK ((destino_cobro = 'factura') = (item_facturacion_id IS NOT NULL)),
    ADD CONSTRAINT fk_recursos_item_facturacion FOREIGN KEY (tenant_id, item_facturacion_id)
        REFERENCES items_facturacion (tenant_id, id);
GRANT UPDATE (destino_cobro, item_facturacion_id) ON recursos TO dad_app;

ALTER TABLE cargos_pendientes DROP CONSTRAINT cargos_pendientes_concepto_check;
ALTER TABLE cargos_pendientes ADD CONSTRAINT cargos_pendientes_concepto_check
    CHECK (concepto IN ('danos', 'penalidad_cancelacion', 'valor_reserva'));
-- Un solo cargo por el valor de cada reserva (aprobarla dos veces no lo duplica).
CREATE UNIQUE INDEX ux_cargo_valor_reserva ON cargos_pendientes (tenant_id, reserva_id)
    WHERE concepto = 'valor_reserva' AND estado <> 'anulado';
