-- Revierte el plan 15 (tributario y documentos electrónicos).

DELETE FROM roles_permisos WHERE permiso LIKE 'tributario.%' OR permiso LIKE 'facturacion.%';
DELETE FROM permisos WHERE codigo LIKE 'tributario.%' OR codigo LIKE 'facturacion.%';

-- Los triggers de solo inserción impiden UPDATE, DELETE y TRUNCATE, no DROP.
DROP TABLE beneficiarios_finales;
DROP TABLE generaciones_exogena;
DROP TABLE eventos_radian;
DROP TABLE facturas_recibidas;
DROP TABLE eventos_documento_electronico;
DROP TABLE documentos_electronicos;
DROP TABLE resoluciones_numeracion;

ALTER TABLE conceptos_cobro DROP COLUMN clasificacion_tributaria;
ALTER TABLE lineas_factura_venta
    DROP CONSTRAINT ck_linea_venta_iva,
    DROP COLUMN clasificacion;

DROP TABLE perfiles_tributarios;
DROP FUNCTION exigir_perfil_borrador();
DROP TABLE campos_formato_exogena;
DROP TABLE formatos_exogena;
DROP TABLE tarifas_ica;
DROP TABLE tarifas_iva;

ALTER TABLE archivos DROP CONSTRAINT archivos_tipo_mime_check;
ALTER TABLE archivos ADD CONSTRAINT archivos_tipo_mime_check CHECK (tipo_mime IN (
    'application/pdf', 'image/jpeg', 'image/png', 'image/webp', 'image/heic', 'text/csv',
    'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')) NOT VALID;

DELETE FROM modulos_habilitados WHERE modulo = 'facturacion_electronica';
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'bienes_comunes', 'control_acceso', 'asambleas',
    'gestion_administrativa', 'publicidad'));
