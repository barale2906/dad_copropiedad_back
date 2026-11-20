DROP FUNCTION entidades_plataforma(timestamptz, uuid, text, text, text, uuid, integer);

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
