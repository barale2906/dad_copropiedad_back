DROP INDEX ix_bienes_privados_codigo_busqueda;
DROP INDEX ix_personas_nombre_busqueda;
DROP FUNCTION normalizar(TEXT);
DROP EXTENSION IF EXISTS pg_trgm;
DROP EXTENSION IF EXISTS unaccent;
