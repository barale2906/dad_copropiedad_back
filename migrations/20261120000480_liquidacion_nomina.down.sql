-- Revierte el plan 16 fase 2 (liquidación de la nómina).

DELETE FROM roles_permisos WHERE permiso IN ('nomina.configurar', 'nomina.novedades.registrar',
                                              'nomina.liquidar', 'nomina.aprobar');
DELETE FROM permisos WHERE codigo IN ('nomina.configurar', 'nomina.novedades.registrar',
                                      'nomina.liquidar', 'nomina.aprobar');

DROP TABLE lineas_liquidacion_nomina;
DROP TABLE liquidaciones_nomina;
DROP FUNCTION exigir_periodo_sin_aprobar();
DROP TABLE anulaciones_novedad_nomina;
DROP TABLE novedades_nomina;
DROP FUNCTION exigir_novedad_en_periodo_abierto();
DROP TABLE periodos_nomina;
DROP FUNCTION proteger_periodo_nomina();
DROP TABLE configuraciones_nomina;
DROP TABLE tramos_laborales;

-- El catálogo es de solo inserción: el trigger se suspende solo para retirar los parámetros de
-- esta fase.
ALTER TABLE parametros_laborales DISABLE TRIGGER parametros_laborales_solo_insercion;
DELETE FROM parametros_laborales WHERE parametro IN (
    'tope_ibc', 'tope_exoneracion_aportes', 'renta_exenta_laboral', 'tope_renta_exenta_laboral',
    'limite_beneficios_retencion', 'tope_beneficios_retencion');
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
        'dias_vacaciones', 'apoyo_aprendiz_lectiva', 'apoyo_aprendiz_productiva'));
