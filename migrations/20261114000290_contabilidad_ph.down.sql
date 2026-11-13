-- Revierte la vertical contable de la PH.

DELETE FROM roles_permisos WHERE permiso LIKE 'cartera.%' OR permiso = 'presupuesto.gestionar';
DELETE FROM permisos WHERE codigo LIKE 'cartera.%' OR codigo = 'presupuesto.gestionar';

REVOKE UPDATE (estado, obligacion_id) ON cargos_pendientes FROM dad_app;
ALTER TABLE cargos_pendientes
    DROP CONSTRAINT fk_cargo_obligacion,
    DROP CONSTRAINT ck_cargo_obligacion,
    DROP COLUMN obligacion_id;

DELETE FROM aplicaciones_pago WHERE obligacion_id IS NOT NULL;
DROP INDEX ix_aplicaciones_obligacion;
ALTER TABLE aplicaciones_pago
    DROP CONSTRAINT fk_aplicacion_obligacion,
    DROP CONSTRAINT ck_aplicacion_destino,
    DROP COLUMN obligacion_id,
    ALTER COLUMN factura_venta_id SET NOT NULL;

-- Los triggers de solo inserción impiden UPDATE, DELETE y TRUNCATE, no DROP.
DROP TABLE ajustes_obligacion;
DROP TABLE obligaciones;
DROP TABLE cuentas_cobro;
DROP TABLE liquidaciones_expensas;
DROP TABLE cuotas_extraordinarias;
DROP TABLE rubros_presupuesto;
DROP FUNCTION exigir_presupuesto_borrador();
DROP TABLE presupuestos;
DROP TABLE politicas_cartera;
DROP TABLE tarifas_concepto_cobro;
DROP TABLE conceptos_cobro;

-- Los cuadros de los módulos de contribución desaparecen con sus módulos.
ALTER TABLE coeficientes_bien_privado DISABLE TRIGGER trg_coeficientes_solo_borrador;
DELETE FROM coeficientes_bien_privado c USING cuadros_coeficientes q
WHERE c.cuadro_id = q.id AND q.modulo_id IS NOT NULL;
ALTER TABLE coeficientes_bien_privado ENABLE TRIGGER trg_coeficientes_solo_borrador;
DELETE FROM cuadros_coeficientes WHERE modulo_id IS NOT NULL;
DROP INDEX ux_cuadro_vigente;
DROP INDEX ux_cuadro_version;
ALTER TABLE cuadros_coeficientes
    DROP CONSTRAINT fk_cuadro_modulo,
    DROP CONSTRAINT ck_cuadro_modulo,
    DROP COLUMN modulo_id,
    ADD CONSTRAINT cuadros_coeficientes_tenant_id_tipo_version_key UNIQUE (tenant_id, tipo, version);
CREATE UNIQUE INDEX ux_cuadro_vigente ON cuadros_coeficientes (tenant_id, tipo)
    WHERE estado = 'vigente';
DROP TABLE modulos_contribucion;

DROP INDEX ux_bienes_referencia_pago;
ALTER TABLE bienes_privados
    DROP CONSTRAINT fk_bienes_privados_pagador,
    DROP COLUMN pagador_persona_id,
    DROP COLUMN referencia_pago;

DELETE FROM modulos_habilitados WHERE modulo = 'contabilidad_ph';
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'bienes_comunes', 'control_acceso', 'asambleas', 'gestion_administrativa',
    'publicidad'));
