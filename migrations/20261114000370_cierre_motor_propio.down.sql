DROP INDEX ix_facturas_nota_debito;
ALTER TABLE facturas_venta
    DROP CONSTRAINT ck_nota_debito_no_es_de_si_misma,
    DROP CONSTRAINT ck_nota_debito_con_concepto,
    DROP CONSTRAINT fk_factura_nota_debito,
    DROP COLUMN concepto_nota_debito,
    DROP COLUMN nota_debito_de;

ALTER TABLE resoluciones_numeracion DROP CONSTRAINT resoluciones_numeracion_tipo_documento_check;
ALTER TABLE resoluciones_numeracion ADD CONSTRAINT resoluciones_numeracion_tipo_documento_check
    CHECK (tipo_documento IN ('factura', 'documento_soporte'));

DROP FUNCTION correo_recepcion_dian(text);
DROP TABLE correos_recepcion_dian;

DELETE FROM envios_set_pruebas WHERE tipo = 'nota_debito';
ALTER TABLE envios_set_pruebas DROP CONSTRAINT envios_set_pruebas_tipo_check;
ALTER TABLE envios_set_pruebas ADD CONSTRAINT envios_set_pruebas_tipo_check
    CHECK (tipo IN ('factura', 'nota_credito'));

DELETE FROM consecutivos
 WHERE serie IN ('evento_radian', 'nota_credito', 'nota_debito', 'nota_ajuste_ds');
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo', 'factura_venta', 'archivo_dian')
           OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

-- Los eventos son de solo inserción: para volver atrás se retiran las aceptaciones tácitas.
ALTER TABLE eventos_radian DISABLE TRIGGER eventos_radian_solo_insercion;
DELETE FROM eventos_radian WHERE documento_id IS NOT NULL;
ALTER TABLE eventos_radian ENABLE TRIGGER eventos_radian_solo_insercion;

DROP INDEX ux_evento_numero;
DROP INDEX ux_evento_documento_tipo;
ALTER TABLE eventos_radian DROP CONSTRAINT eventos_radian_tipo_check;
ALTER TABLE eventos_radian ADD CONSTRAINT eventos_radian_tipo_check CHECK (tipo IN (
    'acuse', 'recibo_bien', 'aceptacion', 'reclamo'));
ALTER TABLE eventos_radian
    DROP CONSTRAINT ck_evento_reclamo_con_concepto,
    DROP CONSTRAINT ck_evento_tacita_propia,
    DROP CONSTRAINT ck_evento_una_referencia,
    DROP CONSTRAINT fk_evento_xml,
    DROP CONSTRAINT fk_evento_documento,
    DROP COLUMN xml_archivo_id,
    DROP COLUMN concepto_reclamo,
    DROP COLUMN numero,
    DROP COLUMN documento_id,
    ALTER COLUMN factura_recibida_id SET NOT NULL;
