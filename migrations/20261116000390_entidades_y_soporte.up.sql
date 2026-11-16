-- Plan 20, fase 2: la plataforma lista y cambia de estado cualquier entidad (PH, empresa o la
-- plataforma), gestiona sus usuarios con permiso y entra como soporte de solo lectura.

INSERT INTO permisos (codigo, ambito) VALUES
    ('plataforma.entidades.usuarios', 'plataforma'),
    ('plataforma.soporte.ver', 'plataforma');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('superusuario'), 'plataforma', p, NULL
FROM unnest(ARRAY['plataforma.entidades.usuarios', 'plataforma.soporte.ver']) AS p;
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.soporte.ver', NULL);

-- Acceso de soporte de una cuenta de la plataforma a una entidad cliente: solo lectura, con
-- motivo y vigencia corta. Tabla de plataforma (sin tenant): la consulta el extractor de sesión.
CREATE TABLE accesos_soporte (
    id          UUID        PRIMARY KEY,
    cuenta_id   UUID        NOT NULL REFERENCES cuentas (id),
    tenant_id   UUID        NOT NULL REFERENCES entidades (id),
    motivo      TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 1 AND 500),
    desde       TIMESTAMPTZ NOT NULL,
    hasta       TIMESTAMPTZ NOT NULL CHECK (hasta > desde),
    creado_en   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_accesos_soporte_vigentes ON accesos_soporte (cuenta_id, tenant_id, hasta DESC);
ALTER TABLE accesos_soporte ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON accesos_soporte
    USING (app_plataforma()) WITH CHECK (app_plataforma());
GRANT SELECT, INSERT ON accesos_soporte TO dad_app;
-- Historial: no se edita ni se borra (el acceso vence solo).
CREATE TRIGGER trg_accesos_soporte_append_only BEFORE UPDATE OR DELETE ON accesos_soporte
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

-- Búsqueda normalizada de entidades por nombre en la consola.
CREATE INDEX ix_entidades_nombre_trgm ON entidades USING gin (normalizar(nombre) gin_trgm_ops);

-- Entidades para la consola de la plataforma: tipo, estado y módulos vigentes. Solo desde una
-- transacción de plataforma (los módulos están bajo RLS por tenant). Por cursor de id.
CREATE FUNCTION entidades_plataforma(
    p_ahora TIMESTAMPTZ, p_id UUID, p_texto TEXT, p_despues UUID, p_limite INTEGER)
RETURNS TABLE (id UUID, nombre TEXT, nit TEXT, tipo TEXT, estado TEXT, modulos TEXT[])
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT app_plataforma() THEN
        RAISE EXCEPTION 'entidades_plataforma exige una transacción de plataforma';
    END IF;
    RETURN QUERY
    SELECT e.id, e.nombre, e.nit,
           CASE WHEN e.es_plataforma THEN 'plataforma'
                WHEN EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = e.id)
                    THEN 'propiedad_horizontal'
                ELSE 'empresa' END,
           e.estado,
           ARRAY(SELECT DISTINCT m.modulo FROM modulos_habilitados m
                 WHERE m.tenant_id = e.id AND m.desde <= p_ahora
                   AND (m.hasta IS NULL OR m.hasta > p_ahora)
                 ORDER BY m.modulo)
    FROM entidades e
    WHERE (p_id IS NULL OR e.id = p_id)
      AND (p_despues IS NULL OR e.id > p_despues)
      AND (p_texto IS NULL OR normalizar(e.nombre) LIKE normalizar(p_texto) ESCAPE '\'
           OR e.nit LIKE p_texto ESCAPE '\')
    ORDER BY e.id
    LIMIT p_limite;
END
$$;
REVOKE ALL ON FUNCTION entidades_plataforma(timestamptz, uuid, text, uuid, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION entidades_plataforma(timestamptz, uuid, text, uuid, integer) TO dad_app;
