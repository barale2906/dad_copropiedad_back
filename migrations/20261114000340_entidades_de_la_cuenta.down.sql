DELETE FROM roles_permisos WHERE permiso = 'plataforma.entidad_propia.designar';
DELETE FROM permisos WHERE codigo = 'plataforma.entidad_propia.designar';

DROP FUNCTION membresias_de_cuenta(uuid);
CREATE FUNCTION membresias_de_cuenta(p_cuenta UUID)
RETURNS TABLE (tenant_id UUID, nombre TEXT, roles TEXT[])
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT c.id, c.nombre, array_agg(DISTINCT r.codigo ORDER BY r.codigo)
    FROM asignaciones_rol a
    JOIN entidades c ON c.id = a.tenant_id
    JOIN roles r         ON r.id = a.rol_id
    WHERE a.cuenta_id = p_cuenta
      AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
    GROUP BY c.id, c.nombre
    ORDER BY c.nombre
$$;
REVOKE ALL ON FUNCTION membresias_de_cuenta(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION membresias_de_cuenta(uuid) TO dad_app;

DROP FUNCTION tipo_entidad(uuid);
DROP FUNCTION designar_entidad_plataforma(uuid);
DROP TRIGGER trg_plataforma_no_copropiedad ON entidades;
DROP TRIGGER trg_copropiedad_no_plataforma ON copropiedades;
DROP FUNCTION exigir_plataforma_no_copropiedad();
DROP INDEX ux_entidad_plataforma;
ALTER TABLE entidades DROP COLUMN es_plataforma;
