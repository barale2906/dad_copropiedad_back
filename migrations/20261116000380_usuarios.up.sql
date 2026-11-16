-- Plan 20, fase 1: los usuarios los crea la plataforma o la entidad (sin registro abierto). La
-- persona define su contraseña con un código que le llega al correo; una misma cuenta trabaja
-- en varias entidades (se reutiliza por correo).

-- Contraseña pendiente: la cuenta existe pero aún no puede iniciar sesión.
ALTER TABLE cuentas
    ALTER COLUMN hash_contrasena DROP NOT NULL,
    ALTER COLUMN contrasena_actualizada_en DROP NOT NULL,
    ADD CONSTRAINT ck_cuentas_contrasena_pendiente
        CHECK ((hash_contrasena IS NULL) = (contrasena_actualizada_en IS NULL));

ALTER TABLE codigos_verificacion DROP CONSTRAINT codigos_verificacion_proposito_check;
ALTER TABLE codigos_verificacion ADD CONSTRAINT codigos_verificacion_proposito_check
    CHECK (proposito IN ('verificar_correo', 'recuperar_contrasena', 'definir_contrasena'));

ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check
    CHECK (tipo IN ('bienes_privados', 'usuarios'));

INSERT INTO permisos (codigo, ambito) VALUES ('plataforma.cuentas.crear', 'plataforma');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.cuentas.crear', NULL);

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER para la entidad de la transacción
-- ---------------------------------------------------------------------
-- Cuenta para un usuario que crea la entidad: la existente con ese correo (una persona trabaja
-- con una sola cuenta en varias entidades) o una nueva con la contraseña pendiente. El permiso
-- lo exige la aplicación; aquí solo se crea o se reconoce la cuenta. `resultado`: `creada`,
-- `existente`, `inactiva` (bloqueada o eliminada) o `usuario_ocupado` (el usuario pedido es de
-- otra cuenta).
CREATE FUNCTION resolver_cuenta_para_entidad(
    p_id UUID, p_nombre TEXT, p_usuario TEXT, p_correo TEXT, p_ahora TIMESTAMPTZ)
RETURNS TABLE (id UUID, resultado TEXT, nombre_mostrar TEXT, pendiente BOOLEAN)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    c RECORD;
BEGIN
    IF app_tenant_id() IS NULL THEN
        RAISE EXCEPTION 'resolver_cuenta_para_entidad exige una entidad en la transacción';
    END IF;
    SELECT k.id, k.estado, k.nombre_mostrar, k.hash_contrasena IS NULL AS pendiente INTO c
    FROM cuentas k WHERE k.correo = lower(p_correo);
    IF FOUND THEN
        RETURN QUERY SELECT c.id,
            CASE WHEN c.estado = 'activa' THEN 'existente' ELSE 'inactiva' END,
            c.nombre_mostrar, c.pendiente;
        RETURN;
    END IF;
    IF p_usuario IS NOT NULL AND EXISTS (SELECT 1 FROM cuentas k WHERE k.usuario = lower(p_usuario)) THEN
        RETURN QUERY SELECT NULL::uuid, 'usuario_ocupado'::text, NULL::text, FALSE;
        RETURN;
    END IF;
    INSERT INTO cuentas (id, nombre_mostrar, usuario, correo, creada_en)
    VALUES (p_id, p_nombre, lower(p_usuario), lower(p_correo), p_ahora);
    RETURN QUERY SELECT p_id, 'creada'::text, p_nombre, TRUE;
END
$$;

-- Código para definir la contraseña de una cuenta pendiente que tiene un rol en la entidad de la
-- transacción (así una entidad no emite códigos para cuentas ajenas). `false` si no aplica.
CREATE FUNCTION emitir_codigo_definir_contrasena(
    p_id UUID, p_cuenta UUID, p_hash TEXT, p_expira TIMESTAMPTZ, p_ahora TIMESTAMPTZ)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    INSERT INTO codigos_verificacion (id, cuenta_id, proposito, hash_codigo, expira_en, creado_en)
    SELECT p_id, k.id, 'definir_contrasena', p_hash, p_expira, p_ahora
    FROM cuentas k
    WHERE k.id = p_cuenta AND k.estado = 'activa' AND k.hash_contrasena IS NULL
      AND EXISTS (SELECT 1 FROM asignaciones_rol a
                  WHERE a.tenant_id = app_tenant_id() AND a.cuenta_id = k.id);
    RETURN FOUND;
END
$$;

-- Usuarios de la entidad de la transacción: cuentas con algún rol vigente allí y sus roles. Solo
-- lo que la gestión de usuarios necesita (nunca hashes ni secretos). Por cursor de id; la
-- búsqueda recorre solo las cuentas de la entidad, por eso no necesita índice propio.
CREATE FUNCTION usuarios_entidad(
    p_ahora TIMESTAMPTZ, p_cuenta UUID, p_texto TEXT, p_despues UUID, p_limite INTEGER)
RETURNS TABLE (id UUID, nombre_mostrar TEXT, usuario TEXT, correo TEXT, estado TEXT,
               contrasena_pendiente BOOLEAN, ultimo_acceso_en TIMESTAMPTZ, roles TEXT[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT k.id, k.nombre_mostrar, k.usuario, k.correo, k.estado, k.hash_contrasena IS NULL,
           k.ultimo_acceso_en,
           ARRAY(SELECT DISTINCT r.codigo FROM asignaciones_rol a JOIN roles r ON r.id = a.rol_id
                 WHERE a.tenant_id = app_tenant_id() AND a.cuenta_id = k.id
                   AND a.vigente_desde <= p_ahora
                   AND (a.vigente_hasta IS NULL OR a.vigente_hasta > p_ahora)
                 ORDER BY r.codigo)
    FROM cuentas k
    WHERE EXISTS (SELECT 1 FROM asignaciones_rol a
                  WHERE a.tenant_id = app_tenant_id() AND a.cuenta_id = k.id
                    AND a.vigente_desde <= p_ahora
                    AND (a.vigente_hasta IS NULL OR a.vigente_hasta > p_ahora))
      AND (p_cuenta IS NULL OR k.id = p_cuenta)
      AND (p_despues IS NULL OR k.id > p_despues)
      AND (p_texto IS NULL OR normalizar(k.nombre_mostrar) LIKE normalizar(p_texto) ESCAPE '\'
           OR k.correo LIKE lower(p_texto) ESCAPE '\' OR k.usuario LIKE lower(p_texto) ESCAPE '\')
    ORDER BY k.id
    LIMIT p_limite
$$;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'resolver_cuenta_para_entidad(uuid, text, text, text, timestamptz)',
        'emitir_codigo_definir_contrasena(uuid, uuid, text, timestamptz, timestamptz)',
        'usuarios_entidad(timestamptz, uuid, text, uuid, integer)'
    ] LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f);
        EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO dad_app', f);
    END LOOP;
END
$$;
