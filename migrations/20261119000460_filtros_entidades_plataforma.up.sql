-- S21 del frontend: la consola de la plataforma filtra las entidades por tipo y estado en la
-- base (antes solo por texto) para que el filtro abarque todas las páginas y no solo lo cargado.

DROP FUNCTION entidades_plataforma(timestamptz, uuid, text, uuid, integer);

CREATE FUNCTION entidades_plataforma(
    p_ahora TIMESTAMPTZ, p_id UUID, p_texto TEXT, p_tipo TEXT, p_estado TEXT, p_despues UUID,
    p_limite INTEGER)
RETURNS TABLE (id UUID, nombre TEXT, nit TEXT, tipo TEXT, estado TEXT, modulos TEXT[])
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT app_plataforma() THEN
        RAISE EXCEPTION 'entidades_plataforma exige una transacción de plataforma';
    END IF;
    RETURN QUERY
    SELECT t.id, t.nombre, t.nit, t.tipo, t.estado,
           ARRAY(SELECT DISTINCT m.modulo FROM modulos_habilitados m
                 WHERE m.tenant_id = t.id AND m.desde <= p_ahora
                   AND (m.hasta IS NULL OR m.hasta > p_ahora)
                 ORDER BY m.modulo)
    FROM (
        SELECT e.id, e.nombre, e.nit,
               CASE WHEN e.es_plataforma THEN 'plataforma'
                    WHEN EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = e.id)
                        THEN 'propiedad_horizontal'
                    ELSE 'empresa' END AS tipo,
               e.estado
        FROM entidades e
        WHERE (p_id IS NULL OR e.id = p_id)
          AND (p_despues IS NULL OR e.id > p_despues)
          AND (p_estado IS NULL OR e.estado = p_estado)
          AND (p_texto IS NULL OR normalizar(e.nombre) LIKE normalizar(p_texto) ESCAPE '\'
               OR e.nit LIKE p_texto ESCAPE '\')
    ) t
    WHERE p_tipo IS NULL OR t.tipo = p_tipo
    ORDER BY t.id
    LIMIT p_limite;
END
$$;
REVOKE ALL ON FUNCTION entidades_plataforma(timestamptz, uuid, text, text, text, uuid, integer)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION entidades_plataforma(timestamptz, uuid, text, text, text, uuid, integer)
    TO dad_app;
