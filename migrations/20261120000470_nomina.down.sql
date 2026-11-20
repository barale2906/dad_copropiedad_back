-- Revierte el plan 16 fase 1 (nómina).

DELETE FROM roles_permisos WHERE permiso LIKE 'nomina.%';
DELETE FROM permisos WHERE codigo LIKE 'nomina.%';

-- Los triggers de solo inserción impiden UPDATE, DELETE y TRUNCATE, no DROP.
DROP TABLE versiones_contrato_laboral;
DROP TABLE contratos_laborales;
DROP FUNCTION exigir_version_contrato();
DROP FUNCTION validar_version_contrato();
DROP FUNCTION proteger_contrato_laboral();
DROP TABLE empleados;
DROP TABLE administradoras_seguridad_social;
DROP TABLE parametros_laborales;

DELETE FROM importaciones WHERE tipo = 'empleados';
ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check
    CHECK (tipo IN ('bienes_privados', 'usuarios'));

DELETE FROM modulos_habilitados WHERE modulo = 'nomina';
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'facturacion_electronica', 'bienes_comunes',
    'control_acceso', 'asambleas', 'gestion_administrativa'));
