-- Desprendibles de nómina (plan 16 fase 3): el portal del empleado en `/yo/nomina` lista sus
-- liquidaciones aprobadas en todas las entidades donde trabaja. Cruza tenants: solo por esta
-- función, que devuelve lo mínimo (sin líneas, cuenta bancaria ni datos de otros empleados).
CREATE FUNCTION desprendibles_de_cuenta(p_cuenta UUID, p_limite INTEGER)
RETURNS TABLE (tenant_id UUID, entidad TEXT, liquidacion_id UUID, periodicidad TEXT,
               desde DATE, hasta DATE, estado TEXT, fecha_pago DATE, devengos NUMERIC,
               deducciones NUMERIC, neto NUMERIC)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT l.tenant_id, e.nombre, l.id, p.periodicidad, p.desde, p.hasta, p.estado,
           p.fecha_pago, l.devengos, l.deducciones, l.neto
    FROM liquidaciones_nomina l
    JOIN periodos_nomina p ON p.id = l.periodo_id AND p.estado IN ('aprobado', 'pagado')
    JOIN empleados em ON em.id = l.empleado_id
    JOIN personas pe ON pe.id = em.persona_id AND pe.tenant_id = l.tenant_id
    JOIN entidades e ON e.id = l.tenant_id
    WHERE pe.cuenta_id = p_cuenta
    ORDER BY p.hasta DESC, e.nombre
    LIMIT least(greatest(p_limite, 1), 120)
$$;
REVOKE ALL ON FUNCTION desprendibles_de_cuenta(UUID, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION desprendibles_de_cuenta(UUID, INTEGER) TO dad_app;
