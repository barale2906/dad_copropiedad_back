-- Listado del archivo de la entidad (solicitud S11 del frontend): más recientes primero y
-- búsqueda por nombre sin tildes ni mayúsculas.
CREATE INDEX ix_archivos_listado ON archivos (tenant_id, id DESC);
CREATE INDEX ix_archivos_nombre_trgm
    ON archivos USING gin (normalizar(nombre_original) gin_trgm_ops);
