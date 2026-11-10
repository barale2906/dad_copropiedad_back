DELETE FROM roles_permisos WHERE permiso = 'comunicados.gestionar';
DELETE FROM permisos WHERE codigo = 'comunicados.gestionar';

CREATE OR REPLACE FUNCTION copropiedades_con_gestion_pendiente()
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT t.tenant_id FROM tickets t
    WHERE (t.estado = 'resuelto' AND t.reabrible_hasta <= now())
       OR (t.vence_en <= now() AND t.alerta_vencimiento_en IS NULL
           AND t.estado NOT IN ('resuelto', 'cerrado', 'rechazado', 'duplicado', 'en_espera_solicitante'))
    UNION
    SELECT p.tenant_id FROM planes_mantenimiento p WHERE p.activo
$$;

DELETE FROM outbox WHERE concepto = 'comunicados';
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check CHECK (concepto IN ('control_acceso'));

DROP FUNCTION destinatarios_comunicado(JSONB, TEXT, TIMESTAMPTZ);
DROP TABLE comunicado_destinatarios;
DROP TABLE comunicado_adjuntos;
DROP TABLE comunicados;
