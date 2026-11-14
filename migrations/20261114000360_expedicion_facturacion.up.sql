-- Expedición completa con el motor propio (plan 17 fase 6): el `AttachedDocument` que se entrega
-- al adquirente (el documento y la respuesta de la DIAN; en contingencia, sin respuesta) y la
-- lista de documentos en contingencia que el worker vuelve a transmitir (dentro de 48 h).

ALTER TABLE documentos_electronicos
    ADD COLUMN adjunto_archivo_id UUID,
    ADD CONSTRAINT fk_documento_adjunto
        FOREIGN KEY (tenant_id, adjunto_archivo_id) REFERENCES archivos (tenant_id, id);

CREATE INDEX ix_documentos_contingencia ON documentos_electronicos (creado_en)
    WHERE estado = 'contingencia';

-- Cruza tenants: solo los identificadores de los documentos en contingencia, los más antiguos
-- primero (los más próximos a vencer el plazo de 48 h de la DIAN).
CREATE FUNCTION documentos_en_contingencia(p_limite INTEGER)
RETURNS TABLE (tenant_id UUID, documento_id UUID, creado_en TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT d.tenant_id, d.id, d.creado_en
    FROM documentos_electronicos d
    WHERE d.estado = 'contingencia'
    ORDER BY d.creado_en, d.id
    LIMIT p_limite
$$;
REVOKE ALL ON FUNCTION documentos_en_contingencia(integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION documentos_en_contingencia(integer) TO dad_app;
