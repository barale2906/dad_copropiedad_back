-- Prestaciones sociales y liquidación del contrato (plan 16 fase 4): novedad de vacaciones
-- disfrutadas y líneas de provisión (informativas: no se pagan ni se descuentan).
ALTER TABLE novedades_nomina DROP CONSTRAINT novedades_nomina_tipo_check;
ALTER TABLE novedades_nomina ADD CONSTRAINT novedades_nomina_tipo_check CHECK (tipo IN (
    'hora_extra_diurna', 'hora_extra_nocturna',
                            'hora_extra_dominical_diurna', 'hora_extra_dominical_nocturna',
                            'recargo_nocturno', 'recargo_dominical', 'recargo_nocturno_dominical',
                            'incapacidad_comun', 'incapacidad_laboral', 'licencia_maternidad',
                            'licencia_paternidad', 'licencia_remunerada', 'licencia_no_remunerada',
                            'ausencia', 'suspension', 'bonificacion_salarial',
                            'bonificacion_no_salarial', 'comision', 'auxilio_no_salarial',
                            'prestamo', 'libranza', 'embargo', 'otra_deduccion', 'vacaciones'));

ALTER TABLE lineas_liquidacion_nomina DROP CONSTRAINT lineas_liquidacion_nomina_clase_check;
ALTER TABLE lineas_liquidacion_nomina ADD CONSTRAINT lineas_liquidacion_nomina_clase_check
    CHECK (clase IN ('devengo', 'deduccion', 'aporte_empleador', 'provision'));

CREATE INDEX ix_liquidaciones_contrato ON liquidaciones_nomina (tenant_id, contrato_id);
