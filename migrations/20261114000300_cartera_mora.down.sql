-- Revierte la fase 5 de la cartera de la PH.

DELETE FROM roles_permisos WHERE permiso = 'cartera.certificados.emitir';
DELETE FROM permisos WHERE codigo = 'cartera.certificados.emitir';

-- Los triggers de solo inserción impiden UPDATE, DELETE y TRUNCATE, no DROP.
DROP TABLE certificados_emitidos;
DROP TABLE cuotas_acuerdo;
DROP TABLE acuerdos_pago;
DROP TABLE causaciones_interes;

-- Las obligaciones de intereses desaparecen con sus causaciones.
DELETE FROM aplicaciones_pago WHERE obligacion_id IN
    (SELECT id FROM obligaciones WHERE origen = 'interes');
DELETE FROM ajustes_obligacion WHERE obligacion_id IN
    (SELECT id FROM obligaciones WHERE origen = 'interes');
DELETE FROM obligaciones WHERE origen = 'interes';
DROP INDEX ux_obligacion_interes;
ALTER TABLE obligaciones
    DROP CONSTRAINT fk_obligacion_causacion,
    DROP CONSTRAINT ck_obligacion_causacion,
    DROP COLUMN causacion_id,
    DROP CONSTRAINT obligaciones_origen_check,
    ADD CONSTRAINT obligaciones_origen_check CHECK (origen IN (
        'liquidacion', 'cargo', 'saldo_inicial', 'manual'));
DROP TABLE causaciones_mora;
DROP TABLE tasas_referencia;
