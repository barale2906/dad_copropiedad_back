DROP FUNCTION usuarios_entidad(timestamptz, uuid, text, uuid, integer);
DROP FUNCTION emitir_codigo_definir_contrasena(uuid, uuid, text, timestamptz, timestamptz);
DROP FUNCTION resolver_cuenta_para_entidad(uuid, text, text, text, timestamptz);

DELETE FROM roles_permisos WHERE permiso = 'plataforma.cuentas.crear';
DELETE FROM permisos WHERE codigo = 'plataforma.cuentas.crear';

DELETE FROM importaciones WHERE tipo = 'usuarios';
ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check CHECK (tipo IN ('bienes_privados'));

DELETE FROM codigos_verificacion WHERE proposito = 'definir_contrasena';
ALTER TABLE codigos_verificacion DROP CONSTRAINT codigos_verificacion_proposito_check;
ALTER TABLE codigos_verificacion ADD CONSTRAINT codigos_verificacion_proposito_check
    CHECK (proposito IN ('verificar_correo', 'recuperar_contrasena'));

-- Las cuentas pendientes quedan sin poder iniciar sesión: '!' nunca verifica como Argon2id.
ALTER TABLE cuentas DROP CONSTRAINT ck_cuentas_contrasena_pendiente;
UPDATE cuentas SET hash_contrasena = '!', contrasena_actualizada_en = creada_en
WHERE hash_contrasena IS NULL;
ALTER TABLE cuentas
    ALTER COLUMN hash_contrasena SET NOT NULL,
    ALTER COLUMN contrasena_actualizada_en SET NOT NULL;
