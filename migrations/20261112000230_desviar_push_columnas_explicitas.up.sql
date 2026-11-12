-- outbox_desviar_push leía la fila completa con `SELECT *` (prohibido en AGENTS.md §8): una
-- columna nueva en `outbox` cambiaba en silencio lo que leía la función. Se leen solo las
-- columnas que usa. CREATE OR REPLACE conserva los permisos (EXECUTE solo para dad_app).
CREATE OR REPLACE FUNCTION outbox_desviar_push(p_id BIGINT, p_linea TEXT, p_destino TEXT, p_payload JSONB)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_tenant    UUID;
    v_destino   TEXT;
    v_prioridad SMALLINT;
    v_concepto  TEXT;
BEGIN
    SELECT o.tenant_id, o.destino, o.prioridad, o.concepto
    INTO v_tenant, v_destino, v_prioridad, v_concepto
    FROM outbox o WHERE o.id = p_id AND o.canal = 'push';
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE outbox SET estado = 'fallido', intentos = intentos + 1,
                      ultimo_error = 'token push inválido: desviado a WhatsApp'
    WHERE id = p_id;
    UPDATE dispositivos_push SET activo = FALSE, actualizado_en = now() WHERE token = v_destino;
    IF p_payload IS NOT NULL THEN
        INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto)
        VALUES (v_tenant, 'whatsapp', v_prioridad, p_linea, p_destino, p_payload, v_concepto);
    END IF;
END
$$;
