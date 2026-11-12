-- Restaura la definición de 20261109000140_control_acceso.
CREATE OR REPLACE FUNCTION outbox_desviar_push(p_id BIGINT, p_linea TEXT, p_destino TEXT, p_payload JSONB)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    o outbox%ROWTYPE;
BEGIN
    SELECT * INTO o FROM outbox WHERE id = p_id AND canal = 'push';
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE outbox SET estado = 'fallido', intentos = intentos + 1,
                      ultimo_error = 'token push inválido: desviado a WhatsApp'
    WHERE id = p_id;
    UPDATE dispositivos_push SET activo = FALSE, actualizado_en = now() WHERE token = o.destino;
    IF p_payload IS NOT NULL THEN
        INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto)
        VALUES (o.tenant_id, 'whatsapp', o.prioridad, p_linea, p_destino, p_payload, o.concepto);
    END IF;
END
$$;
