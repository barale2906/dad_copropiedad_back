-- Revierte el outbox multicanal. Los mensajes de canales distintos de WhatsApp se ELIMINAN:
-- el esquema anterior no puede representarlos (exige `linea`).

DROP FUNCTION outbox_marcar_enviado(BIGINT, TEXT);
DROP FUNCTION outbox_reclamar(TEXT[], INTEGER);

DELETE FROM outbox WHERE canal <> 'whatsapp';

ALTER TABLE outbox RENAME COLUMN referencia_salida TO wamid_salida;

CREATE FUNCTION outbox_marcar_enviado(p_id BIGINT, p_wamid TEXT) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    UPDATE outbox
    SET estado = 'enviado', enviado_en = now(), wamid_salida = p_wamid, intentos = intentos + 1
    WHERE id = p_id
$$;


CREATE FUNCTION outbox_reclamar(p_limite INTEGER)
RETURNS TABLE (id BIGINT, tenant_id UUID, prioridad SMALLINT, linea TEXT, destino TEXT,
               payload JSONB, intentos INTEGER)
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.id, o.tenant_id, o.prioridad, o.linea, o.destino, o.payload, o.intentos
    FROM outbox o
    WHERE o.estado = 'pendiente' AND o.disponible_en <= now()
    ORDER BY o.prioridad, o.disponible_en, o.id
    LIMIT p_limite
    FOR UPDATE SKIP LOCKED
$$;

DROP INDEX idx_outbox_pendientes;
CREATE INDEX idx_outbox_pendientes ON outbox (prioridad, disponible_en, id) WHERE estado = 'pendiente';

COMMENT ON COLUMN outbox.destino IS NULL;
ALTER TABLE outbox DROP CONSTRAINT ck_outbox_linea_solo_whatsapp;
ALTER TABLE outbox ALTER COLUMN linea SET NOT NULL;
ALTER TABLE outbox DROP COLUMN canal;

REVOKE ALL ON FUNCTION outbox_reclamar(INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION outbox_reclamar(INTEGER) TO dad_app;
GRANT EXECUTE ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) TO dad_app;
