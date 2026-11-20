-- Revierte los complementos de la nómina.
DROP TABLE deducciones_retencion;
DROP FUNCTION proteger_deduccion_retencion();

ALTER TABLE parametros_laborales DISABLE TRIGGER parametros_laborales_solo_insercion;
DELETE FROM parametros_laborales WHERE parametro IN (
        'porcentaje_dependientes', 'tope_dependientes', 'tope_intereses_vivienda',
        'tope_medicina_prepagada', 'limite_aportes_voluntarios', 'tope_aportes_voluntarios');
ALTER TABLE parametros_laborales ENABLE TRIGGER parametros_laborales_solo_insercion;
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
        'limite_beneficios_retencion', 'tope_beneficios_retencion'));

DELETE FROM importaciones WHERE tipo = 'novedades_nomina';
ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check
    CHECK (tipo IN ('bienes_privados', 'usuarios', 'empleados'));

ALTER TABLE versiones_contrato_laboral DROP CONSTRAINT fk_version_centro_costo,
    DROP COLUMN centro_costo_id;
