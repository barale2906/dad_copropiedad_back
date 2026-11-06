-- =====================================================================
-- DAD_COPROPIEDAD · Esquema inicial (multi-PH, votaciones, registro, outbox)
-- =====================================================================
-- Garantías que da la propia base de datos:
--   1. Multi-tenant: toda tabla de negocio lleva `tenant_id` (= copropiedad) y RLS. La app
--      se conecta con el rol `dad_app` (sin BYPASSRLS, no es dueño de las tablas) y cada
--      transacción fija `set_config('app.tenant_id', <uuid>, true)` (= SET LOCAL).
--      Las FK son compuestas (tenant_id, id): una fila no puede apuntar a otro tenant.
--   2. Unicidad: un inmueble participa una vez por pregunta      -> UNIQUE (pregunta_id, inmueble_id)
--   3. Idempotencia: reintentos y webhooks repetidos de Meta     -> UNIQUE (pregunta_id, idempotency_key)
--                                                                   UNIQUE (wamid)
--   4. Solo preguntas `abierta` reciben participaciones (trigger con FOR SHARE: sin carreras
--      contra el cierre).
--   5. Secreto del voto: en preguntas secretas `votos` no referencia inmueble ni participación
--      (trigger) y su id es UUID v4 (CHECK). `votos` no tiene columna de hora.
--   6. participaciones, votos y auditoria son append-only (triggers + permisos).
--   7. Coeficientes en NUMERIC exacto (nunca float).
-- =====================================================================

-- ---------------------------------------------------------------------
-- Rol de la aplicación (global al clúster). En desarrollo lo crea con LOGIN
-- docker/postgres/init/01-roles.sql; aquí solo se garantiza que exista.
-- ---------------------------------------------------------------------
DO $$
BEGIN
    CREATE ROLE dad_app NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
EXCEPTION
    WHEN duplicate_object OR unique_violation THEN NULL;  -- ya existe (o carrera entre pruebas)
END
$$;

-- Tenant de la transacción actual (NULL si no se fijó: RLS no devuelve filas).
CREATE FUNCTION app_tenant_id() RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.tenant_id', true), '')::uuid
$$;

-- Rechazo genérico para tablas append-only.
CREATE FUNCTION rechazar_modificacion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'la tabla % es de solo inserción', TG_TABLE_NAME
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

-- ---------------------------------------------------------------------
-- Plataforma (sin tenant)
-- ---------------------------------------------------------------------
-- Líneas de WhatsApp. Usar una sola línea o una por asamblea es configuración, no rediseño.
CREATE TABLE lineas_whatsapp (
    id               UUID PRIMARY KEY,
    phone_number_id  TEXT        NOT NULL UNIQUE,     -- id de la línea en Meta
    numero_visible   TEXT        NOT NULL,            -- p. ej. +57 300 000 0000
    nombre           TEXT        NOT NULL,
    limite_mps       INTEGER     NOT NULL DEFAULT 80 CHECK (limite_mps > 0),
    activa           BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------
-- Tenant: copropiedad (PH)
-- ---------------------------------------------------------------------
CREATE TABLE copropiedades (
    id         UUID PRIMARY KEY,                      -- = tenant_id
    nombre     TEXT        NOT NULL,
    nit        TEXT        UNIQUE,
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE inmuebles (
    id           UUID PRIMARY KEY,
    tenant_id    UUID          NOT NULL REFERENCES copropiedades (id),
    codigo       TEXT          NOT NULL,              -- p. ej. "T1-101"
    coeficiente  NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    activo       BOOLEAN       NOT NULL DEFAULT TRUE,
    creado_en    TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id)
);

-- Propietarios y apoderados (datos personales: Ley 1581 de 2012).
CREATE TABLE personas (
    id         UUID PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    nombre     TEXT        NOT NULL,
    documento  TEXT,
    telefono   TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),   -- E.164 sin '+'
    correo     TEXT,
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, documento),
    UNIQUE (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Reuniones y registro
-- ---------------------------------------------------------------------
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

-- Quién representa a cada inmueble en una reunión. Métodos: enlace prellenado con botones,
-- prerregistro de la administración o QR en la puerta.
CREATE TABLE inscripciones (
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id     UUID        NOT NULL,
    inmueble_id    UUID        NOT NULL,
    persona_id     UUID        NOT NULL,
    rol            TEXT        NOT NULL CHECK (rol IN ('propietario', 'apoderado')),
    metodo         TEXT        NOT NULL CHECK (metodo IN ('enlace', 'prerregistro', 'qr_puerta')),
    telefono       TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),  -- para enrutar WhatsApp
    token_hash     BYTEA       UNIQUE,      -- SHA-256 del token del enlace/QR; nunca el token
    inscrito_en    TIMESTAMPTZ NOT NULL DEFAULT now(),
    asistencia_en  TIMESTAMPTZ,             -- confirmación de asistencia (QR en puerta / botón)
    UNIQUE (reunion_id, inmueble_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_id)  REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, inmueble_id) REFERENCES inmuebles (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)  REFERENCES personas  (tenant_id, id)
);
CREATE INDEX idx_inscripciones_telefono ON inscripciones (telefono) WHERE telefono IS NOT NULL;

-- ---------------------------------------------------------------------
-- Preguntas y opciones
-- ---------------------------------------------------------------------
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
    UNIQUE (tenant_id, pregunta_id, id),           -- destino de la FK compuesta desde `votos`
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Participaciones: QUIÉN votó, cuándo, por qué canal y con qué peso (para el acta).
-- Nunca QUÉ votó.
-- ---------------------------------------------------------------------
CREATE TABLE participaciones (
    id               UUID PRIMARY KEY,
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id      UUID          NOT NULL,
    inmueble_id      UUID          NOT NULL,
    coeficiente      NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1), -- copia al votar
    canal            TEXT          NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial')),
    emitido_por      UUID          NOT NULL,       -- persona (propietario o apoderado)
    apoderado_id     UUID,                         -- = emitido_por cuando vota un apoderado
    idempotency_key  TEXT          NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    wamid            TEXT,                         -- id del mensaje de WhatsApp
    registrado_en    TIMESTAMPTZ   NOT NULL DEFAULT now(),
    CONSTRAINT uq_participacion_inmueble     UNIQUE (pregunta_id, inmueble_id),
    CONSTRAINT uq_participacion_idempotencia UNIQUE (pregunta_id, idempotency_key),
    CONSTRAINT uq_participacion_wamid        UNIQUE (wamid),
    UNIQUE (tenant_id, id),
    CHECK ((canal = 'whatsapp') = (wamid IS NOT NULL)),
    FOREIGN KEY (tenant_id, pregunta_id)  REFERENCES preguntas (tenant_id, id),
    FOREIGN KEY (tenant_id, inmueble_id)  REFERENCES inmuebles (tenant_id, id),
    FOREIGN KEY (tenant_id, emitido_por)  REFERENCES personas  (tenant_id, id),
    FOREIGN KEY (tenant_id, apoderado_id) REFERENCES personas  (tenant_id, id)
);

-- Solo preguntas abiertas. FOR SHARE bloquea un cierre concurrente hasta el COMMIT.
-- Nota para el adaptador: buscar primero la participación por idempotency_key/wamid para que un
-- reintento tras el cierre devuelva el resultado original en lugar de este error.
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
            USING ERRCODE = 'V0001';   -- el adaptador lo traduce a PreguntaNoAbierta
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_participaciones_validar
    BEFORE INSERT ON participaciones
    FOR EACH ROW EXECUTE FUNCTION validar_participacion();

-- ---------------------------------------------------------------------
-- Votos: QUÉ se votó y con qué peso. Sin columna de hora.
--   - Secreto : inmueble_id y participacion_id NULL.
--   - Nominal : ambos obligatorios.
-- ---------------------------------------------------------------------
CREATE TABLE votos (
    id                UUID PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id       UUID          NOT NULL,
    opcion_id         UUID          NOT NULL,
    coeficiente       NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    inmueble_id       UUID,
    participacion_id  UUID          UNIQUE,
    CONSTRAINT ck_voto_uuid_v4 CHECK (substring(id::text, 15, 1) = '4'),  -- v7 revelaría la hora
    CHECK ((inmueble_id IS NULL) = (participacion_id IS NULL)),
    FOREIGN KEY (tenant_id, pregunta_id, opcion_id) REFERENCES opciones (tenant_id, pregunta_id, id),
    FOREIGN KEY (tenant_id, inmueble_id)      REFERENCES inmuebles       (tenant_id, id),
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

    IF v_tipo = 'secreto' AND NEW.inmueble_id IS NOT NULL THEN
        RAISE EXCEPTION 'un voto secreto no puede referenciar inmueble ni participación'
            USING ERRCODE = 'check_violation';
    ELSIF v_tipo = 'nominal' AND NEW.inmueble_id IS NULL THEN
        RAISE EXCEPTION 'un voto nominal debe referenciar inmueble y participación'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_votos_validar
    BEFORE INSERT ON votos
    FOR EACH ROW EXECUTE FUNCTION validar_voto();

-- ---------------------------------------------------------------------
-- Outbox: mensajes salientes grabados en la misma transacción que el hecho que los origina.
-- Prioridad: 1 confirmación de voto · 2 pregunta nueva · 3 recordatorio.
-- ---------------------------------------------------------------------
CREATE TABLE outbox (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    prioridad      SMALLINT    NOT NULL CHECK (prioridad BETWEEN 1 AND 9),
    linea          TEXT        NOT NULL,      -- phone_number_id de la línea emisora
    destino        TEXT        NOT NULL,      -- teléfono E.164 sin '+'
    payload        JSONB       NOT NULL,      -- nunca la opción de un voto secreto
    estado         TEXT        NOT NULL DEFAULT 'pendiente'
                   CHECK (estado IN ('pendiente', 'enviado', 'fallido')),
    intentos       INTEGER     NOT NULL DEFAULT 0,
    disponible_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    creado_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    enviado_en     TIMESTAMPTZ,
    wamid_salida   TEXT,
    ultimo_error   TEXT
);
CREATE INDEX idx_outbox_pendientes ON outbox (prioridad, disponible_en, id) WHERE estado = 'pendiente';

-- ---------------------------------------------------------------------
-- Auditoría (append-only). Nunca registrar la opción de un voto secreto.
-- ---------------------------------------------------------------------
CREATE TABLE auditoria (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    ocurrido_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_id     UUID,                         -- NULL = sistema
    accion       TEXT        NOT NULL,         -- p. ej. 'pregunta.abrir', 'voto.registrar'
    entidad      TEXT        NOT NULL,
    entidad_id   UUID,
    resultado    TEXT        NOT NULL CHECK (resultado IN ('ok', 'rechazado', 'error')),
    detalle      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    request_id   TEXT
);
CREATE INDEX idx_auditoria_entidad ON auditoria (tenant_id, entidad, entidad_id);
CREATE INDEX idx_auditoria_fecha   ON auditoria (tenant_id, ocurrido_en);

-- ---------------------------------------------------------------------
-- Append-only: participaciones, votos, auditoria
-- ---------------------------------------------------------------------
CREATE TRIGGER trg_participaciones_inmutable BEFORE UPDATE OR DELETE ON participaciones
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_participaciones_sin_truncate BEFORE TRUNCATE ON participaciones
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_votos_inmutable BEFORE UPDATE OR DELETE ON votos
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_votos_sin_truncate BEFORE TRUNCATE ON votos
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_auditoria_inmutable BEFORE UPDATE OR DELETE ON auditoria
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_auditoria_sin_truncate BEFORE TRUNCATE ON auditoria
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

-- ---------------------------------------------------------------------
-- Row-Level Security por tenant
-- (ENABLE sin FORCE: el dueño de las tablas —usado solo para migraciones y para las funciones
--  SECURITY DEFINER de abajo— no queda filtrado; `dad_app` sí.)
-- ---------------------------------------------------------------------
ALTER TABLE copropiedades ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON copropiedades
    USING (id = app_tenant_id()) WITH CHECK (id = app_tenant_id());

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'inmuebles', 'personas', 'reuniones', 'inscripciones', 'preguntas', 'opciones',
        'participaciones', 'votos', 'outbox', 'auditoria'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER (cruzan tenants de forma controlada; solo devuelven lo mínimo)
-- ---------------------------------------------------------------------
-- Enrutamiento del gateway: reuniones activas de esta línea en que el teléfono está inscrito.
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

-- Consumo del outbox: llamar dentro de una transacción; las filas quedan bloqueadas
-- (FOR UPDATE SKIP LOCKED) hasta el COMMIT, así varios workers no envían lo mismo.
CREATE FUNCTION outbox_reclamar(p_limite INTEGER)
RETURNS TABLE (id BIGINT, tenant_id UUID, prioridad SMALLINT, linea TEXT, destino TEXT,
               payload JSONB, intentos INTEGER)
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.id, o.tenant_id, o.prioridad, o.linea, o.destino, o.payload, o.intentos
    FROM outbox o
    WHERE o.estado = 'pendiente' AND o.disponible_en <= now()
    ORDER BY o.prioridad, o.disponible_en, o.id
    LIMIT p_limite
    FOR UPDATE SKIP LOCKED
$$;

CREATE FUNCTION outbox_marcar_enviado(p_id BIGINT, p_wamid TEXT) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    UPDATE outbox
    SET estado = 'enviado', enviado_en = now(), wamid_salida = p_wamid, intentos = intentos + 1
    WHERE id = p_id
$$;

-- Reprograma con espera `p_espera`; tras `p_max_intentos` queda en 'fallido'.
CREATE FUNCTION outbox_marcar_fallido(p_id BIGINT, p_error TEXT, p_espera INTERVAL,
                                      p_max_intentos INTEGER) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    UPDATE outbox
    SET intentos      = intentos + 1,
        ultimo_error  = left(p_error, 1000),
        estado        = CASE WHEN intentos + 1 >= p_max_intentos THEN 'fallido' ELSE 'pendiente' END,
        disponible_en = now() + p_espera
    WHERE id = p_id
$$;

-- ---------------------------------------------------------------------
-- Permisos del rol de la aplicación (mínimo privilegio)
-- ---------------------------------------------------------------------
GRANT USAGE ON SCHEMA public TO dad_app;
GRANT SELECT ON lineas_whatsapp TO dad_app;
GRANT SELECT, UPDATE ON copropiedades TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE
    ON inmuebles, personas, reuniones, inscripciones, preguntas, opciones TO dad_app;
GRANT SELECT, INSERT ON participaciones, votos, outbox, auditoria TO dad_app;
GRANT USAGE ON SEQUENCE outbox_id_seq, auditoria_id_seq TO dad_app;

REVOKE ALL ON FUNCTION reuniones_activas_por_telefono(TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION outbox_reclamar(INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION outbox_marcar_fallido(BIGINT, TEXT, INTERVAL, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION reuniones_activas_por_telefono(TEXT, TEXT) TO dad_app;
GRANT EXECUTE ON FUNCTION outbox_reclamar(INTEGER) TO dad_app;
GRANT EXECUTE ON FUNCTION outbox_marcar_enviado(BIGINT, TEXT) TO dad_app;
GRANT EXECUTE ON FUNCTION outbox_marcar_fallido(BIGINT, TEXT, INTERVAL, INTEGER) TO dad_app;
