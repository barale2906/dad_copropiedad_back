DELETE FROM roles_permisos WHERE permiso IN (
    'agrupaciones.editar', 'bienes_privados.coeficientes.activar', 'titulares.ver',
    'titulares.editar', 'importaciones.ejecutar');
DELETE FROM permisos WHERE codigo IN (
    'agrupaciones.editar', 'bienes_privados.coeficientes.activar', 'titulares.ver',
    'titulares.editar', 'importaciones.ejecutar');

DROP TABLE importaciones;
DELETE FROM archivos WHERE tipo_mime IN (
    'text/csv', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet');
ALTER TABLE archivos DROP CONSTRAINT archivos_tipo_mime_check;
ALTER TABLE archivos ADD CONSTRAINT archivos_tipo_mime_check CHECK (tipo_mime IN (
    'application/pdf', 'image/jpeg', 'image/png', 'image/webp', 'image/heic'));

DROP TABLE titularidades;
DROP FUNCTION validar_titulares_bien();

ALTER TABLE personas
    DROP CONSTRAINT fk_personas_representante_legal,
    DROP CONSTRAINT ck_representante_legal,
    DROP CONSTRAINT uq_personas_cuenta,
    DROP COLUMN representante_legal_id,
    DROP COLUMN cuenta_id,
    DROP COLUMN tipo;

DROP TRIGGER trg_bienes_privados_coeficiente ON bienes_privados;
DROP FUNCTION exigir_coeficiente_del_cuadro();
DROP TABLE coeficientes_bien_privado;
DROP FUNCTION exigir_cuadro_borrador();
DROP TABLE cuadros_coeficientes;
DROP FUNCTION aplicar_cuadro_vigente();
DROP FUNCTION validar_cuadro_coeficientes();
DROP TABLE usos_exclusivos;

-- Los bienes privados sin coeficiente no caben en el esquema anterior.
DELETE FROM bienes_privados WHERE coeficiente IS NULL;
ALTER TABLE bienes_privados
    DROP CONSTRAINT fk_bienes_privados_principal,
    DROP CONSTRAINT fk_bienes_privados_agrupacion,
    DROP CONSTRAINT ck_bien_principal_distinto,
    DROP COLUMN actualizado_en,
    DROP COLUMN bien_principal_id,
    DROP COLUMN uso,
    DROP COLUMN cedula_catastral,
    DROP COLUMN matricula_inmobiliaria,
    DROP COLUMN area_privada_m2,
    DROP COLUMN piso,
    DROP COLUMN numero,
    DROP COLUMN agrupacion_id,
    DROP COLUMN tipo,
    ALTER COLUMN coeficiente SET NOT NULL;

DROP TABLE agrupaciones;
DROP TABLE excepciones_punto_atencion;
DROP TABLE franjas_punto_atencion;
DROP TABLE puntos_atencion;
DROP FUNCTION cargos_vigentes();

ALTER TABLE copropiedades
    DROP CONSTRAINT fk_copropiedades_certificado,
    DROP CONSTRAINT fk_copropiedades_reglamento,
    DROP CONSTRAINT ck_copropiedades_nit,
    DROP COLUMN actualizado_en,
    DROP COLUMN plantilla_codigo,
    DROP COLUMN estado,
    DROP COLUMN zona_horaria,
    DROP COLUMN certificado_archivo_id,
    DROP COLUMN reglamento_archivo_id,
    DROP COLUMN matricula_matriz,
    DROP COLUMN escritura_fecha,
    DROP COLUMN notaria,
    DROP COLUMN escritura_numero,
    DROP COLUMN sitio_web,
    DROP COLUMN correo,
    DROP COLUMN telefono,
    DROP COLUMN estrato,
    DROP COLUMN longitud,
    DROP COLUMN latitud,
    DROP COLUMN barrio,
    DROP COLUMN departamento,
    DROP COLUMN municipio_dane,
    DROP COLUMN direccion,
    DROP COLUMN forma,
    DROP COLUMN tipo,
    DROP COLUMN razon_social;
