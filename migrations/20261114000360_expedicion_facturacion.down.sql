DROP FUNCTION documentos_en_contingencia(integer);
DROP INDEX ix_documentos_contingencia;
ALTER TABLE documentos_electronicos
    DROP CONSTRAINT fk_documento_adjunto,
    DROP COLUMN adjunto_archivo_id;
