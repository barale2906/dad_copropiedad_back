-- Reintentos seguros de la API (plan 10, fase 6): con la cabecera `Idempotency-Key`, un POST
-- repetido por la misma cuenta devuelve la respuesta original durante 24 h. Tabla de plataforma
-- (`app.plataforma = on`): la cuenta no pertenece a una sola copropiedad.

CREATE TABLE peticiones_idempotentes (
    cuenta_id UUID        NOT NULL,
    clave     TEXT        NOT NULL CHECK (char_length(clave) BETWEEN 1 AND 200),
    -- SHA-256 de método, ruta y cuerpo: la misma clave con otra petición se rechaza.
    huella    TEXT        NOT NULL CHECK (huella ~ '^[0-9a-f]{64}$'),
    -- NULL mientras la primera petición está en curso.
    estado    SMALLINT    CHECK (estado BETWEEN 100 AND 499),
    tipo      TEXT,
    cuerpo    BYTEA       CHECK (octet_length(cuerpo) <= 1048576),
    creada_en TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (cuenta_id, clave)
);
CREATE INDEX ix_peticiones_idempotentes_creada ON peticiones_idempotentes (creada_en);
COMMENT ON COLUMN peticiones_idempotentes.cuerpo IS
    'dato_personal: respuesta original de la API (puede contener datos de personas); se purga a las 24 h';

ALTER TABLE peticiones_idempotentes ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON peticiones_idempotentes
    USING (app_plataforma()) WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE, DELETE ON peticiones_idempotentes TO dad_app;
