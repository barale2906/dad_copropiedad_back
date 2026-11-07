-- Búsqueda sin tildes ni mayúsculas (plan 01 §8): "jose alvarez" encuentra "José Álvarez".
-- unaccent y pg_trgm son extensiones "trusted": las puede crear el dueño de la base de datos
-- sin ser superusuario.
CREATE EXTENSION IF NOT EXISTS unaccent;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Inmutable para poder indexarla. Se nombra el diccionario con su esquema para que el
-- resultado no dependa del search_path de quien la llame.
CREATE FUNCTION normalizar(texto TEXT)
RETURNS TEXT
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
AS $$ SELECT lower(public.unaccent('public.unaccent'::regdictionary, texto)) $$;

-- Búsqueda por fragmentos (`LIKE '%…%'` o similitud `%`) sobre el texto normalizado.
CREATE INDEX ix_personas_nombre_busqueda
    ON personas USING gin (normalizar(nombre) gin_trgm_ops);
CREATE INDEX ix_bienes_privados_codigo_busqueda
    ON bienes_privados USING gin (normalizar(codigo) gin_trgm_ops);
