-- Cierre del motor propio (plan 17 fase 7).
--
-- 1. Eventos RADIAN con el motor propio (anexo técnico 1.9 §6.5).
-- Cada evento es un `ApplicationResponse` firmado con su número (único por entidad), su CUDE (en
-- `id_externo`) y su XML en el almacén. Los eventos 030–033 se refieren a una factura recibida;
-- la aceptación tácita (034) la emite el facturador sobre su propia factura electrónica: la
-- misma tabla sirve a los dos casos (exactamente una referencia).

ALTER TABLE eventos_radian
    ALTER COLUMN factura_recibida_id DROP NOT NULL,
    ADD COLUMN documento_id      UUID,
    ADD COLUMN numero            TEXT CHECK (char_length(numero) BETWEEN 1 AND 50),
    ADD COLUMN concepto_reclamo  TEXT CHECK (concepto_reclamo IN (
        'inconsistencias', 'no_entregada_total', 'no_entregada_parcial', 'servicio_no_prestado')),
    ADD COLUMN xml_archivo_id    UUID,
    ADD CONSTRAINT fk_evento_documento
        FOREIGN KEY (tenant_id, documento_id) REFERENCES documentos_electronicos (tenant_id, id),
    ADD CONSTRAINT fk_evento_xml
        FOREIGN KEY (tenant_id, xml_archivo_id) REFERENCES archivos (tenant_id, id),
    ADD CONSTRAINT ck_evento_una_referencia
        CHECK (num_nonnulls(factura_recibida_id, documento_id) = 1),
    -- La aceptación tácita es la única sobre la factura propia.
    ADD CONSTRAINT ck_evento_tacita_propia
        CHECK ((tipo = 'aceptacion_tacita') = (documento_id IS NOT NULL)),
    ADD CONSTRAINT ck_evento_reclamo_con_concepto
        CHECK (tipo <> 'reclamo' OR concepto_reclamo IS NOT NULL OR numero IS NULL);

ALTER TABLE eventos_radian DROP CONSTRAINT eventos_radian_tipo_check;
ALTER TABLE eventos_radian ADD CONSTRAINT eventos_radian_tipo_check CHECK (tipo IN (
    'acuse', 'recibo_bien', 'aceptacion', 'reclamo', 'aceptacion_tacita'));

CREATE UNIQUE INDEX ux_evento_documento_tipo ON eventos_radian (tenant_id, documento_id, tipo)
    WHERE documento_id IS NOT NULL;
CREATE UNIQUE INDEX ux_evento_numero ON eventos_radian (tenant_id, numero)
    WHERE numero IS NOT NULL;

-- 2. Consecutivos anuales por entidad de los eventos y de las notas (crédito, débito y de
--    ajuste al documento soporte): las notas no usan resolución de numeración, pero cada una
--    lleva su número propio (prefijo del tipo y del año, p. ej. `NC26` + consecutivo).
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo', 'factura_venta', 'archivo_dian', 'evento_radian',
                     'nota_credito', 'nota_debito', 'nota_ajuste_ds')
           OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

-- 3. El set de pruebas de habilitación lleva también notas débito.
ALTER TABLE envios_set_pruebas DROP CONSTRAINT envios_set_pruebas_tipo_check;
ALTER TABLE envios_set_pruebas ADD CONSTRAINT envios_set_pruebas_tipo_check
    CHECK (tipo IN ('factura', 'nota_credito', 'nota_debito'));

-- 4. Correos de recepción que los facturadores electrónicos registraron ante la DIAN
--    (`GetExchangeEmails`, anexo §7.16). La DIAN entrega la lista completa: se sincroniza a
--    diario con las credenciales de la entidad de la plataforma y cada entidad la consulta por
--    NIT al entregar sus facturas (antes que el correo de facturación del tercero).
CREATE TABLE correos_recepcion_dian (
    nit            TEXT        PRIMARY KEY CHECK (nit ~ '^[0-9]{3,15}$'),
    correo         TEXT        NOT NULL CHECK (char_length(correo) BETWEEN 3 AND 320),
    actualizado_en TIMESTAMPTZ NOT NULL
);
COMMENT ON TABLE correos_recepcion_dian IS
    'Plataforma: correo de recepción de facturas registrado en la DIAN por cada facturador.';
COMMENT ON COLUMN correos_recepcion_dian.correo IS
    'dato_personal: correo de recepción (de una persona natural cuando factura con su cédula)';
ALTER TABLE correos_recepcion_dian ENABLE ROW LEVEL SECURITY;
CREATE POLICY gestionar ON correos_recepcion_dian USING (app_plataforma())
    WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE ON correos_recepcion_dian TO dad_app;

-- Cruza ámbitos: una entidad consulta solo el correo de un NIT.
CREATE FUNCTION correo_recepcion_dian(p_nit TEXT)
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT correo FROM correos_recepcion_dian WHERE nit = p_nit
$$;
REVOKE ALL ON FUNCTION correo_recepcion_dian(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION correo_recepcion_dian(text) TO dad_app;

-- 5. Contingencia del facturador (tipo 03, anexo §12.1): el talonario de papel tiene su propia
--    resolución de numeración (sin clave técnica) y cada factura de papel se transcribe después
--    como factura electrónica tipo 03 con el mismo número.
ALTER TABLE resoluciones_numeracion DROP CONSTRAINT resoluciones_numeracion_tipo_documento_check;
ALTER TABLE resoluciones_numeracion ADD CONSTRAINT resoluciones_numeracion_tipo_documento_check
    CHECK (tipo_documento IN ('factura', 'documento_soporte', 'contingencia'));

-- 6. Nota débito de una factura de venta: es un documento por cobrar con la misma forma que la
--    factura (cliente, líneas, IVA, saldo, recaudo, pago manual y contabilización), así que vive
--    en `facturas_venta` con la factura que ajusta y su concepto de la DIAN; entra a la cartera
--    por edades y se abona con recibos de caja como cualquier factura.
ALTER TABLE facturas_venta
    ADD COLUMN nota_debito_de        UUID,
    ADD COLUMN concepto_nota_debito  TEXT CHECK (concepto_nota_debito IN (
        'intereses', 'gastos_por_cobrar', 'cambio_del_valor', 'otro')),
    ADD CONSTRAINT fk_factura_nota_debito
        FOREIGN KEY (tenant_id, nota_debito_de) REFERENCES facturas_venta (tenant_id, id),
    ADD CONSTRAINT ck_nota_debito_con_concepto
        CHECK ((nota_debito_de IS NULL) = (concepto_nota_debito IS NULL)),
    ADD CONSTRAINT ck_nota_debito_no_es_de_si_misma CHECK (nota_debito_de <> id);
CREATE INDEX ix_facturas_nota_debito ON facturas_venta (tenant_id, nota_debito_de)
    WHERE nota_debito_de IS NOT NULL;
