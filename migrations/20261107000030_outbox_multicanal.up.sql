-- =====================================================================
-- Outbox multicanal (plan 01, fase 4): además de WhatsApp, correo y (más adelante) push.
--   - `canal` dice por dónde sale el mensaje; `destino` es el teléfono, el correo o el token.
--   - `linea` (número de WhatsApp emisor) solo aplica al canal whatsapp.
--   - `wamid_salida` pasa a `referencia_salida`: id que devuelve el proveedor de cualquier canal.
--   - Cada worker reclama solo los canales que sabe enviar (`outbox_reclamar(p_canales, …)`).
-- =====================================================================

ALTER TABLE outbox
    ADD COLUMN canal TEXT NOT NULL DEFAULT 'whatsapp'
        CHECK (canal IN ('whatsapp', 'correo', 'push'));
-- El valor por defecto solo sirve para las filas existentes: quien encola debe decir el canal.
ALTER TABLE outbox ALTER COLUMN canal DROP DEFAULT;

ALTER TABLE outbox ALTER COLUMN linea DROP NOT NULL;
ALTER TABLE outbox
    ADD CONSTRAINT ck_outbox_linea_solo_whatsapp CHECK ((canal = 'whatsapp') = (linea IS NOT NULL));

ALTER TABLE outbox RENAME COLUMN wamid_salida TO referencia_salida;

COMMENT ON COLUMN outbox.destino IS
    'whatsapp: teléfono E.164 sin +; correo: dirección; push: token del dispositivo. Dato personal.';

DROP INDEX idx_outbox_pendientes;
CREATE INDEX idx_outbox_pendientes
    ON outbox (canal, prioridad, disponible_en, id) WHERE estado = 'pendiente';

-- Reclamar por canal. Llamar dentro de una transacción: las filas quedan bloqueadas
-- (FOR UPDATE SKIP LOCKED) hasta el COMMIT, así varios workers no envían lo mismo.
DROP FUNCTION outbox_reclamar(INTEGER);
CREATE FUNCTION outbox_reclamar(p_canales TEXT[], p_limite INTEGER)
RETURNS TABLE (id BIGINT, tenant_id UUID, canal TEXT, prioridad SMALLINT, linea TEXT,
               destino TEXT, payload JSONB, intentos INTEGER)
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.id, o.tenant_id, o.canal, o.prioridad, o.linea, o.destino, o.payload, o.intentos
    FROM outbox o
    WHERE o.estado = 'pendiente'
      AND o.disponible_en <= now()
      AND o.canal = ANY (p_canales)
    ORDER BY o.prioridad, o.disponible_en, o.id
    LIMIT p_limite
    FOR UPDATE SKIP LOCKED
$$;

-- Cambia el nombre del parámetro (no se permite con CREATE OR REPLACE).
DROP FUNCTION outbox_marcar_enviado(BIGINT, TEXT);
CREATE FUNCTION outbox_marcar_enviado(p_id BIGINT, p_referencia TEXT) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    UPDATE outbox
    SET estado = 'enviado', enviado_en = now(), referencia_salida = p_referencia,
        intentos = intentos + 1
    WHERE id = p_id
$$;

REVOKE ALL ON FUNCTION outbox_reclamar(TEXT[], INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION outbox_reclamar(TEXT[], INTEGER) TO dad_app;
GRANT EXECUTE ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) TO dad_app;
