-- Revierte 20261108000130_asambleas: vuelve al esquema de votación inicial (sin datos).

DELETE FROM roles_permisos
WHERE permiso IN ('asambleas.registrar', 'asambleas.votos.registrar', 'plataforma.whatsapp.gestionar');
DELETE FROM roles WHERE id = uuid_rol_plantilla('operador_asamblea');
DELETE FROM permisos
WHERE codigo IN ('asambleas.registrar', 'asambleas.votos.registrar', 'plataforma.whatsapp.gestionar');

DROP FUNCTION uso_lineas_whatsapp();
DROP FUNCTION tarifas_whatsapp_vigentes(TEXT, TEXT, DATE);
DROP FUNCTION mensajes_whatsapp_reunion(UUID);
DROP FUNCTION estados_entrega(BIGINT[]);
DROP FUNCTION actualizar_calidad_linea(TEXT, BOOLEAN, TEXT);
DROP FUNCTION registrar_estado_whatsapp(TEXT, TEXT, TEXT, TEXT, BOOLEAN, INTEGER, TIMESTAMPTZ);
DROP FUNCTION copropiedades_con_asambleas_pendientes();
DROP FUNCTION asistencias_por_telefono(TEXT);
DROP FUNCTION elegir_linea_whatsapp();

DROP TABLE tarifas_whatsapp;
DROP TABLE whatsapp_estados;

ALTER TABLE lineas_whatsapp ADD COLUMN activa BOOLEAN NOT NULL DEFAULT TRUE;
UPDATE lineas_whatsapp SET activa = (estado <> 'pausada');
ALTER TABLE lineas_whatsapp
    DROP COLUMN actualizado_en,
    DROP COLUMN franquicia_servicio_mes,
    DROP COLUMN calidad,
    DROP COLUMN estado;

CREATE OR REPLACE FUNCTION linea_whatsapp_predeterminada()
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT phone_number_id FROM lineas_whatsapp WHERE activa ORDER BY creado_en, id LIMIT 1
$$;

DROP INDEX ix_outbox_reunion;
ALTER TABLE outbox DROP COLUMN reunion_id;

DROP TABLE votos;
DROP TABLE participaciones;
DROP FUNCTION validar_participacion();
DROP TABLE fotos_quorum;
DROP TABLE pregunta_habilitados;
DROP TABLE candidatos;
DROP TABLE opciones;
DROP TABLE preguntas;
DROP TABLE envios_convocatoria;
DROP TABLE rotulos;
DROP TABLE asistente_presencias;
DROP TABLE asistente_bienes;
DROP TABLE poderes;
DROP TABLE asistentes;
DROP TABLE morosos_reunion;
DROP TABLE padron_reunion;
DROP TABLE documentos_reunion;
DROP TABLE puntos_orden_dia;
DROP TABLE reuniones;

-- Esquema de votación inicial (20261106025538 con el renombre de 20261106224835).
CREATE TABLE reuniones (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID        NOT NULL REFERENCES copropiedades (id),
    titulo             TEXT        NOT NULL,
    tipo               TEXT        NOT NULL CHECK (tipo IN ('ordinaria', 'extraordinaria')),
    convocatoria       TEXT        NOT NULL DEFAULT 'primera'
                       CHECK (convocatoria IN ('primera', 'segunda')),
    estado             TEXT        NOT NULL DEFAULT 'convocada'
                       CHECK (estado IN ('convocada', 'instalada', 'cerrada', 'cancelada')),
    fecha_programada   TIMESTAMPTZ NOT NULL,
    instalada_en       TIMESTAMPTZ,
    cerrada_en         TIMESTAMPTZ,
    linea_whatsapp_id  UUID        REFERENCES lineas_whatsapp (id),
    creado_en          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    CHECK (cerrada_en IS NULL OR (instalada_en IS NOT NULL AND cerrada_en >= instalada_en))
);
CREATE INDEX idx_reuniones_estado ON reuniones (tenant_id, estado);

CREATE TABLE inscripciones (
    id               UUID PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id       UUID        NOT NULL,
    bien_privado_id  UUID        NOT NULL,
    persona_id       UUID        NOT NULL,
    rol              TEXT        NOT NULL CHECK (rol IN ('propietario', 'apoderado')),
    metodo           TEXT        NOT NULL CHECK (metodo IN ('enlace', 'prerregistro', 'qr_puerta')),
    telefono         TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    token_hash       BYTEA       UNIQUE,
    inscrito_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    asistencia_en    TIMESTAMPTZ,
    CONSTRAINT inscripciones_reunion_id_bien_privado_id_key UNIQUE (reunion_id, bien_privado_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_id)      REFERENCES reuniones (tenant_id, id),
    CONSTRAINT inscripciones_tenant_id_bien_privado_id_fkey
        FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)      REFERENCES personas  (tenant_id, id)
);
CREATE INDEX idx_inscripciones_telefono ON inscripciones (telefono) WHERE telefono IS NOT NULL;

CREATE TABLE preguntas (
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id     UUID        NOT NULL,
    orden          INTEGER     NOT NULL,
    enunciado      TEXT        NOT NULL,
    tipo_voto      TEXT        NOT NULL CHECK (tipo_voto IN ('nominal', 'secreto')),
    regla_mayoria  TEXT        NOT NULL CHECK (regla_mayoria IN ('simple', 'calificada_70')),
    estado         TEXT        NOT NULL DEFAULT 'borrador'
                   CHECK (estado IN ('borrador', 'abierta', 'cerrada', 'anulada')),
    abierta_en     TIMESTAMPTZ,
    cerrada_en     TIMESTAMPTZ,
    creado_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (reunion_id, orden),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id),
    CHECK (cerrada_en IS NULL OR (abierta_en IS NOT NULL AND cerrada_en >= abierta_en))
);

CREATE TABLE opciones (
    id           UUID PRIMARY KEY,
    tenant_id    UUID    NOT NULL REFERENCES copropiedades (id),
    pregunta_id  UUID    NOT NULL,
    orden        INTEGER NOT NULL,
    texto        TEXT    NOT NULL,
    UNIQUE (pregunta_id, orden),
    UNIQUE (tenant_id, pregunta_id, id),
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);

CREATE TABLE participaciones (
    id               UUID PRIMARY KEY,
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id      UUID          NOT NULL,
    bien_privado_id  UUID          NOT NULL,
    coeficiente      NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    canal            TEXT          NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial')),
    emitido_por      UUID          NOT NULL,
    apoderado_id     UUID,
    idempotency_key  TEXT          NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    wamid            TEXT,
    registrado_en    TIMESTAMPTZ   NOT NULL DEFAULT now(),
    CONSTRAINT uq_participacion_bien_privado UNIQUE (pregunta_id, bien_privado_id),
    CONSTRAINT uq_participacion_idempotencia UNIQUE (pregunta_id, idempotency_key),
    CONSTRAINT uq_participacion_wamid        UNIQUE (wamid),
    UNIQUE (tenant_id, id),
    CHECK ((canal = 'whatsapp') = (wamid IS NOT NULL)),
    FOREIGN KEY (tenant_id, pregunta_id)  REFERENCES preguntas (tenant_id, id),
    CONSTRAINT participaciones_tenant_id_bien_privado_id_fkey
        FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, emitido_por)  REFERENCES personas  (tenant_id, id),
    FOREIGN KEY (tenant_id, apoderado_id) REFERENCES personas  (tenant_id, id)
);

CREATE FUNCTION validar_participacion() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_estado TEXT;
BEGIN
    SELECT estado INTO v_estado
    FROM preguntas
    WHERE id = NEW.pregunta_id AND tenant_id = NEW.tenant_id
    FOR SHARE;

    IF v_estado IS DISTINCT FROM 'abierta' THEN
        RAISE EXCEPTION 'la pregunta % no está abierta (estado: %)',
            NEW.pregunta_id, coalesce(v_estado, 'inexistente')
            USING ERRCODE = 'V0001';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_participaciones_validar
    BEFORE INSERT ON participaciones
    FOR EACH ROW EXECUTE FUNCTION validar_participacion();

CREATE TABLE votos (
    id                UUID PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id       UUID          NOT NULL,
    opcion_id         UUID          NOT NULL,
    coeficiente       NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    bien_privado_id   UUID,
    participacion_id  UUID          UNIQUE,
    CONSTRAINT ck_voto_uuid_v4 CHECK (substring(id::text, 15, 1) = '4'),
    CHECK ((bien_privado_id IS NULL) = (participacion_id IS NULL)),
    FOREIGN KEY (tenant_id, pregunta_id, opcion_id) REFERENCES opciones (tenant_id, pregunta_id, id),
    CONSTRAINT votos_tenant_id_bien_privado_id_fkey
        FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, participacion_id) REFERENCES participaciones (tenant_id, id)
);
CREATE INDEX idx_votos_conteo ON votos (pregunta_id, opcion_id) INCLUDE (coeficiente);

CREATE FUNCTION validar_voto() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_tipo TEXT;
BEGIN
    SELECT tipo_voto INTO v_tipo
    FROM preguntas
    WHERE id = NEW.pregunta_id AND tenant_id = NEW.tenant_id;

    IF v_tipo = 'secreto' AND NEW.bien_privado_id IS NOT NULL THEN
        RAISE EXCEPTION 'un voto secreto no puede referenciar bien privado ni participación'
            USING ERRCODE = 'check_violation';
    ELSIF v_tipo = 'nominal' AND NEW.bien_privado_id IS NULL THEN
        RAISE EXCEPTION 'un voto nominal debe referenciar bien privado y participación'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_votos_validar
    BEFORE INSERT ON votos
    FOR EACH ROW EXECUTE FUNCTION validar_voto();

CREATE TRIGGER trg_participaciones_inmutable BEFORE UPDATE OR DELETE ON participaciones
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_participaciones_sin_truncate BEFORE TRUNCATE ON participaciones
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_votos_inmutable BEFORE UPDATE OR DELETE ON votos
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_votos_sin_truncate BEFORE TRUNCATE ON votos
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'reuniones', 'inscripciones', 'preguntas', 'opciones', 'participaciones', 'votos'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

CREATE FUNCTION reuniones_activas_por_telefono(p_linea TEXT, p_telefono TEXT)
RETURNS TABLE (tenant_id UUID, reunion_id UUID, titulo TEXT, estado TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT r.tenant_id, r.id, r.titulo, r.estado
    FROM inscripciones i
    JOIN reuniones r       ON r.tenant_id = i.tenant_id AND r.id = i.reunion_id
    JOIN lineas_whatsapp l ON l.id = r.linea_whatsapp_id
    WHERE i.telefono = p_telefono
      AND l.phone_number_id = p_linea
      AND l.activa
      AND r.estado IN ('convocada', 'instalada')
$$;

GRANT SELECT, INSERT, UPDATE, DELETE ON reuniones, inscripciones, preguntas, opciones TO dad_app;
GRANT SELECT, INSERT ON participaciones, votos TO dad_app;
REVOKE ALL ON FUNCTION reuniones_activas_por_telefono(TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION reuniones_activas_por_telefono(TEXT, TEXT) TO dad_app;
