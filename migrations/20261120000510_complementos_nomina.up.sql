-- Complementos de la nómina (plan 16): centro de costo en el contrato (reparte el gasto en la
-- contabilidad), importación de novedades y deducciones de la retención en la fuente que declara
-- el empleado (dependientes, intereses de vivienda, medicina prepagada y aportes voluntarios).

ALTER TABLE versiones_contrato_laboral ADD COLUMN centro_costo_id UUID,
    ADD CONSTRAINT fk_version_centro_costo FOREIGN KEY (tenant_id, centro_costo_id)
        REFERENCES centros_costo (tenant_id, id);

ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check
    CHECK (tipo IN ('bienes_privados', 'usuarios', 'empleados', 'novedades_nomina'));

ALTER TABLE parametros_laborales DROP CONSTRAINT parametros_laborales_parametro_check;
ALTER TABLE parametros_laborales ADD CONSTRAINT parametros_laborales_parametro_check
    CHECK (parametro IN (
        'salario_minimo', 'auxilio_transporte', 'tope_auxilio_transporte',
        'jornada_maxima_semanal', 'inicio_jornada_nocturna', 'fin_jornada_nocturna',
        'recargo_nocturno', 'hora_extra_diurna', 'hora_extra_nocturna',
        'recargo_dominical_festivo', 'salud_empleador', 'salud_empleado', 'pension_empleador',
        'pension_empleado', 'caja_compensacion', 'icbf', 'sena', 'arl_clase_1', 'arl_clase_2',
        'arl_clase_3', 'arl_clase_4', 'arl_clase_5', 'salario_integral_minimo',
        'factor_salario_integral', 'limite_pagos_no_salariales', 'dias_incapacidad_empleador',
        'incapacidad_eps', 'dias_cesantias', 'intereses_cesantias', 'dias_prima',
        'dias_vacaciones', 'apoyo_aprendiz_lectiva', 'apoyo_aprendiz_productiva', 'tope_ibc',
        'tope_exoneracion_aportes', 'renta_exenta_laboral', 'tope_renta_exenta_laboral',
        'limite_beneficios_retencion', 'tope_beneficios_retencion',
        'porcentaje_dependientes', 'tope_dependientes', 'tope_intereses_vivienda',
        'tope_medicina_prepagada', 'limite_aportes_voluntarios', 'tope_aportes_voluntarios'));

-- Lo que el empleado declara con sus soportes. Una deducción se termina (`vigente_hasta`); no
-- se borra ni se edita su valor.
CREATE TABLE deducciones_retencion (
    id                  UUID          PRIMARY KEY,
    tenant_id           UUID          NOT NULL REFERENCES entidades (id),
    empleado_id         UUID          NOT NULL,
    tipo                TEXT          NOT NULL CHECK (tipo IN (
                            'dependientes', 'intereses_vivienda', 'medicina_prepagada',
                            'aportes_voluntarios')),
    -- Valor mensual (cero en dependientes: es un porcentaje del ingreso).
    valor_mensual       NUMERIC(18,2) NOT NULL CHECK (valor_mensual >= 0),
    vigente_desde       DATE          NOT NULL,
    vigente_hasta       DATE          CHECK (vigente_hasta >= vigente_desde),
    soporte_archivo_id  UUID,
    creado_por          UUID,
    creado_en           TIMESTAMPTZ   NOT NULL,
    CONSTRAINT ck_deduccion_valor CHECK ((tipo = 'dependientes') = (valor_mensual = 0)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, empleado_id) REFERENCES empleados (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_deducciones_empleado ON deducciones_retencion (tenant_id, empleado_id);
COMMENT ON COLUMN deducciones_retencion.tipo IS
    'dato_personal: beneficio tributario declarado (dependientes, vivienda, salud)';
COMMENT ON COLUMN deducciones_retencion.valor_mensual IS 'dato_personal: valor declarado';

-- Solo se termina: lo demás no cambia.
CREATE FUNCTION proteger_deduccion_retencion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'una deducción no se borra: se termina' USING ERRCODE = 'V0010';
    END IF;
    IF OLD.vigente_hasta IS NOT NULL OR NEW.tipo <> OLD.tipo
       OR NEW.valor_mensual <> OLD.valor_mensual OR NEW.vigente_desde <> OLD.vigente_desde
       OR NEW.empleado_id <> OLD.empleado_id THEN
        RAISE EXCEPTION 'una deducción solo se termina una vez' USING ERRCODE = 'V0010';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_proteger_deduccion BEFORE UPDATE OR DELETE ON deducciones_retencion
    FOR EACH ROW EXECUTE FUNCTION proteger_deduccion_retencion();

ALTER TABLE deducciones_retencion ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON deducciones_retencion
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
GRANT SELECT, INSERT, UPDATE ON deducciones_retencion TO dad_app;
