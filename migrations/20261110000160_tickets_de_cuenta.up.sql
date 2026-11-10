-- Bloque 2 (plan 10, fase 1): «mis solicitudes en todas mis PH». Tickets que radicó una cuenta
-- (directamente o como persona vinculada) en las copropiedades activas. Cruza tenants: solo por
-- esta función, que devuelve lo mínimo (sin descripción ni mensajes).
CREATE FUNCTION tickets_de_cuenta(p_cuenta UUID, p_limite INTEGER)
RETURNS TABLE (tenant_id UUID, copropiedad TEXT, ticket_id UUID, anio SMALLINT,
               consecutivo INTEGER, tipo TEXT, estado TEXT, asunto TEXT,
               creado_en TIMESTAMPTZ, actualizado_en TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT t.tenant_id, c.nombre, t.id, t.anio, t.consecutivo, t.tipo, t.estado, t.asunto,
           t.creado_en, t.actualizado_en
    FROM tickets t
    JOIN copropiedades c ON c.id = t.tenant_id AND c.estado = 'activa'
    WHERE t.solicitante_cuenta_id = p_cuenta
       OR t.solicitante_persona_id IN (SELECT p.id FROM personas p
                                       WHERE p.tenant_id = t.tenant_id AND p.cuenta_id = p_cuenta)
    ORDER BY t.actualizado_en DESC
    LIMIT least(greatest(p_limite, 1), 100)
$$;
REVOKE ALL ON FUNCTION tickets_de_cuenta(UUID, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION tickets_de_cuenta(UUID, INTEGER) TO dad_app;
