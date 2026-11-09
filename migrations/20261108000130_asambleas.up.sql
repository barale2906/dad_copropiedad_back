-- Asambleas y votaciones (plan 06, ADR 0010): convocatoria, padrón, asistentes que representan
-- varios bienes privados, poderes, morosos, presencias con retiros, rótulos de código de barras,
-- orden del día, preguntas (sí/no, selección única o múltiple, elección por cociente
-- electoral), votos nominales o reservados, fotos del quórum, acta y acuses de WhatsApp.
--
-- Reemplaza el esquema de votación inicial, que no tiene datos reales (decisión del
-- 2026-11-08): `inscripciones` (un bien = una persona) da paso a `asistentes` +
-- `asistente_bienes`, y el tipo de voto `secreto` desaparece (ADR 0010: no existe el secreto
-- absoluto; `reservado` guarda el vínculo y solo lo consulta quien tiene permiso).
--
-- Garantías de la base de datos (AGENTS.md §9):
--   1. Un bien privado vota una vez por pregunta: UNIQUE (pregunta_id, bien_privado_id).
--   2. Idempotencia: el voto de un asistente cubre todos sus bienes con la misma clave; un
--      reintento se reconoce por (pregunta_id, asistente_id, idempotency_key).
--   3. Solo preguntas `abierta` y solo bienes habilitados al abrirla (presentes y sin
--      restricción por mora) reciben votos: trigger `validar_participacion` con FOR SHARE.
--   4. Un bien está en un solo asistente a la vez; un asistente tiene a lo sumo una presencia
--      abierta y un rótulo vigente.
--   5. `participaciones`, `votos`, `fotos_quorum` y `envios_convocatoria` son append-only.

DROP FUNCTION reuniones_activas_por_telefono(TEXT, TEXT);
DROP TABLE votos;
DROP FUNCTION validar_voto();
DROP TABLE participaciones;
DROP FUNCTION validar_participacion();
DROP TABLE opciones;
DROP TABLE preguntas;
DROP TABLE inscripciones;
DROP TABLE reuniones;

-- ---------------------------------------------------------------------
-- Reuniones (asamblea general de propietarios o consulta por escrito, art. 43)
-- ---------------------------------------------------------------------
CREATE TABLE reuniones (
    id                     UUID        PRIMARY KEY,
    tenant_id              UUID        NOT NULL REFERENCES copropiedades (id),
    titulo                 TEXT        NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 200),
    tipo                   TEXT        NOT NULL CHECK (tipo IN ('ordinaria', 'extraordinaria')),
    convocatoria           TEXT        NOT NULL DEFAULT 'primera'
                           CHECK (convocatoria IN ('primera', 'segunda', 'derecho_propio')),
    modalidad              TEXT        NOT NULL DEFAULT 'presencial'
                           CHECK (modalidad IN ('presencial', 'virtual', 'mixta', 'escrita')),
    estado                 TEXT        NOT NULL DEFAULT 'borrador'
                           CHECK (estado IN ('borrador', 'convocada', 'en_registro', 'instalada',
                                             'suspendida', 'cerrada', 'cancelada')),
    fecha_programada       TIMESTAMPTZ NOT NULL,
    lugar                  TEXT        CHECK (char_length(lugar) <= 300),
    -- Enlace de la videollamada (Google Meet u otro proveedor); la votación es siempre aquí.
    enlace_video           TEXT        CHECK (char_length(enlace_video) <= 500),
    -- Consulta por escrito: fecha límite para responder (art. 43: un mes).
    plazo_respuesta        TIMESTAMPTZ,
    -- Segunda convocatoria (art. 41): la reunión de primera que no tuvo quórum.
    reunion_origen_id      UUID,
    -- Configuración de la asamblea (la define cada PH al crearla).
    base_computo           TEXT        NOT NULL DEFAULT 'coeficiente'
                           CHECK (base_computo IN ('coeficiente', 'unidades')),
    max_bienes_apoderado   SMALLINT    CHECK (max_bienes_apoderado > 0),
    -- Cargos que no pueden ser apoderados (códigos de rol plantilla).
    apoderados_prohibidos  TEXT[]      NOT NULL DEFAULT ARRAY['administrador', 'revisor_fiscal']::TEXT[],
    exigir_soporte_poder   BOOLEAN     NOT NULL DEFAULT FALSE,
    efecto_mora            TEXT        NOT NULL DEFAULT 'informativo'
                           CHECK (efecto_mora IN ('informativo', 'sin_voto', 'sin_voto_ni_quorum')),
    temporizador_segundos  INTEGER     CHECK (temporizador_segundos BETWEEN 10 AND 86400),
    convocada_en           TIMESTAMPTZ,
    registro_abierto_en    TIMESTAMPTZ,
    instalada_en           TIMESTAMPTZ,
    cerrada_en             TIMESTAMPTZ,
    cancelada_en           TIMESTAMPTZ,
    motivo_cancelacion     TEXT        CHECK (char_length(motivo_cancelacion) <= 500),
    -- Acta: se genera en PDF; las firmas se suben escaneadas (sin firma electrónica).
    acta_estado            TEXT        NOT NULL DEFAULT 'pendiente'
                           CHECK (acta_estado IN ('pendiente', 'generada', 'aprobada', 'publicada')),
    acta_archivo_id        UUID,
    acta_firmada_archivo_id UUID,
    acta_generada_en       TIMESTAMPTZ,
    acta_aprobada_en       TIMESTAMPTZ,
    acta_publicada_en      TIMESTAMPTZ,
    creada_por             UUID,
    creado_en              TIMESTAMPTZ NOT NULL DEFAULT now(),
    actualizado_en         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_origen_id)       REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, acta_archivo_id)         REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, acta_firmada_archivo_id) REFERENCES archivos (tenant_id, id),
    CHECK ((modalidad = 'escrita') = (plazo_respuesta IS NOT NULL)),
    CHECK (cerrada_en IS NULL OR instalada_en IS NULL OR cerrada_en >= instalada_en)
);
CREATE INDEX ix_reuniones_estado ON reuniones (tenant_id, estado, fecha_programada);

-- Orden del día: los puntos avanzan pendiente → en_curso → terminado (o aplazado).
CREATE TABLE puntos_orden_dia (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id    UUID        NOT NULL,
    orden         SMALLINT    NOT NULL CHECK (orden > 0),
    titulo        TEXT        NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 300),
    descripcion   TEXT        CHECK (char_length(descripcion) <= 4000),
    estado        TEXT        NOT NULL DEFAULT 'pendiente'
                  CHECK (estado IN ('pendiente', 'en_curso', 'terminado', 'aplazado')),
    iniciado_en   TIMESTAMPTZ,
    terminado_en  TIMESTAMPTZ,
    UNIQUE (reunion_id, orden),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id)
);

-- Documentos anexos a la convocatoria (estados financieros, presupuesto, informes).
CREATE TABLE documentos_reunion (
    tenant_id   UUID NOT NULL REFERENCES copropiedades (id),
    reunion_id  UUID NOT NULL,
    archivo_id  UUID NOT NULL,
    tipo        TEXT NOT NULL CHECK (tipo IN ('estados_financieros', 'presupuesto', 'informe',
                                              'reglamento', 'otro')),
    titulo      TEXT NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 200),
    PRIMARY KEY (reunion_id, archivo_id),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Padrón: foto de los bienes privados al convocar (coeficiente del cuadro vigente y
-- representante ante la PH). Los coeficientes no cambian con una reunión instalada.
-- ---------------------------------------------------------------------
CREATE TABLE padron_reunion (
    tenant_id                 UUID          NOT NULL REFERENCES copropiedades (id),
    reunion_id                UUID          NOT NULL,
    bien_privado_id           UUID          NOT NULL,
    coeficiente               NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    representante_persona_id  UUID,
    PRIMARY KEY (reunion_id, bien_privado_id),
    FOREIGN KEY (tenant_id, reunion_id)               REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)          REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, representante_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_padron_representante ON padron_reunion (tenant_id, representante_persona_id);

-- Morosos de la asamblea: listado que entrega la PH (este módulo no depende de contabilidad).
-- El efecto lo fija la reunión (`efecto_mora`).
CREATE TABLE morosos_reunion (
    tenant_id        UUID NOT NULL REFERENCES copropiedades (id),
    reunion_id       UUID NOT NULL,
    bien_privado_id  UUID NOT NULL,
    PRIMARY KEY (reunion_id, bien_privado_id),
    FOREIGN KEY (reunion_id, bien_privado_id) REFERENCES padron_reunion (reunion_id, bien_privado_id)
);

-- ---------------------------------------------------------------------
-- Asistentes: la persona presente (propietario o apoderado) con sus medios y los bienes que
-- representa. Un voto del asistente aplica a todos sus bienes.
-- ---------------------------------------------------------------------
CREATE TABLE asistentes (
    id                      UUID        PRIMARY KEY,
    tenant_id               UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id              UUID        NOT NULL,
    persona_id              UUID        NOT NULL,
    metodo                  TEXT        NOT NULL CHECK (metodo IN ('mesa', 'prerregistro', 'enlace',
                                                                    'consulta')),
    telefono                TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    -- Línea por la que se le escribe (la ventana de 24 h es por par línea–usuario).
    linea_whatsapp          TEXT,
    whatsapp_confirmado_en  TIMESTAMPTZ,
    -- Plantilla `confirmar_asistencia` encolada (para mostrar a la mesa si no llegó).
    confirmacion_outbox_id  BIGINT,
    -- SHA-256 de la papeleta virtual (enlace personal); nunca el token.
    token_hash              BYTEA       UNIQUE,
    registrado_por          UUID,
    creado_en               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (reunion_id, persona_id),
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, reunion_id, id),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_asistentes_telefono ON asistentes (telefono) WHERE telefono IS NOT NULL;
COMMENT ON COLUMN asistentes.telefono IS 'dato_personal: teléfono de WhatsApp del asistente';

-- Poderes (Ley 675, art. 37): el apoderado representa al bien del poderdante.
CREATE TABLE poderes (
    id                     UUID        PRIMARY KEY,
    tenant_id              UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id             UUID        NOT NULL,
    bien_privado_id        UUID        NOT NULL,
    poderdante_persona_id  UUID,
    apoderado_persona_id   UUID        NOT NULL,
    soporte_archivo_id     UUID,
    estado                 TEXT        NOT NULL DEFAULT 'pendiente'
                           CHECK (estado IN ('pendiente', 'verificado', 'rechazado', 'revocado')),
    motivo                 TEXT        CHECK (char_length(motivo) <= 500),
    registrado_por         UUID,
    registrado_en          TIMESTAMPTZ NOT NULL,
    revisado_por           UUID,
    revisado_en            TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (reunion_id, bien_privado_id)     REFERENCES padron_reunion (reunion_id, bien_privado_id),
    FOREIGN KEY (tenant_id, poderdante_persona_id) REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, apoderado_persona_id)  REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id)    REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_poder_vigente ON poderes (reunion_id, bien_privado_id)
    WHERE estado IN ('pendiente', 'verificado');

-- Bienes que representa cada asistente, con vigencia (retiros y reasignaciones).
CREATE TABLE asistente_bienes (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id       UUID        NOT NULL,
    asistente_id     UUID        NOT NULL,
    bien_privado_id  UUID        NOT NULL,
    calidad          TEXT        NOT NULL CHECK (calidad IN ('propietario', 'apoderado')),
    poder_id         UUID,
    desde            TIMESTAMPTZ NOT NULL,
    hasta            TIMESTAMPTZ CHECK (hasta >= desde),
    CHECK ((calidad = 'apoderado') = (poder_id IS NOT NULL)),
    FOREIGN KEY (tenant_id, reunion_id, asistente_id) REFERENCES asistentes (tenant_id, reunion_id, id),
    FOREIGN KEY (reunion_id, bien_privado_id) REFERENCES padron_reunion (reunion_id, bien_privado_id),
    FOREIGN KEY (tenant_id, poder_id)         REFERENCES poderes (tenant_id, id)
);
CREATE UNIQUE INDEX ux_bien_en_un_asistente ON asistente_bienes (reunion_id, bien_privado_id)
    WHERE hasta IS NULL;
CREATE INDEX ix_asistente_bienes ON asistente_bienes (tenant_id, asistente_id) WHERE hasta IS NULL;

-- Periodos de presencia: entrada, salida con motivo y reingreso (nuevo periodo).
CREATE TABLE asistente_presencias (
    id              UUID        PRIMARY KEY,
    tenant_id       UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id      UUID        NOT NULL,
    asistente_id    UUID        NOT NULL,
    entrada_en      TIMESTAMPTZ NOT NULL,
    salida_en       TIMESTAMPTZ CHECK (salida_en >= entrada_en),
    motivo_salida   TEXT        CHECK (motivo_salida IN ('se_retiro', 'delego_poder',
                                                         'problema_tecnico', 'otro')),
    detalle_salida  TEXT        CHECK (char_length(detalle_salida) <= 300),
    registrada_por  UUID,
    CHECK ((salida_en IS NULL) = (motivo_salida IS NULL)),
    FOREIGN KEY (tenant_id, reunion_id, asistente_id) REFERENCES asistentes (tenant_id, reunion_id, id)
);
CREATE UNIQUE INDEX ux_presencia_abierta ON asistente_presencias (asistente_id) WHERE salida_en IS NULL;
CREATE INDEX ix_presencias_reunion ON asistente_presencias (tenant_id, reunion_id) WHERE salida_en IS NULL;

-- Rótulos genéricos preimpresos (Code 128) que la mesa liga a un asistente en el momento.
CREATE TABLE rotulos (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id    UUID        NOT NULL,
    codigo        TEXT        NOT NULL CHECK (codigo ~ '^[A-Z0-9-]{1,32}$'),
    asistente_id  UUID        NOT NULL,
    asignado_en   TIMESTAMPTZ NOT NULL,
    liberado_en   TIMESTAMPTZ CHECK (liberado_en >= asignado_en),
    FOREIGN KEY (tenant_id, reunion_id, asistente_id) REFERENCES asistentes (tenant_id, reunion_id, id)
);
CREATE UNIQUE INDEX ux_rotulo_vigente ON rotulos (reunion_id, codigo) WHERE liberado_en IS NULL;
CREATE UNIQUE INDEX ux_rotulo_por_asistente ON rotulos (asistente_id) WHERE liberado_en IS NULL;

-- Constancia de la convocatoria: a quién y por qué canal se envió (estado en `outbox` y en los
-- acuses de WhatsApp).
CREATE TABLE envios_convocatoria (
    id           BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    reunion_id   UUID        NOT NULL,
    persona_id   UUID        NOT NULL,
    canal        TEXT        NOT NULL CHECK (canal IN ('whatsapp', 'correo')),
    outbox_id    BIGINT      NOT NULL,
    creado_en    TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_envios_convocatoria ON envios_convocatoria (tenant_id, reunion_id);

-- ---------------------------------------------------------------------
-- Preguntas, opciones y candidatos
-- ---------------------------------------------------------------------
CREATE TABLE preguntas (
    id                     UUID         PRIMARY KEY,
    tenant_id              UUID         NOT NULL REFERENCES copropiedades (id),
    reunion_id             UUID         NOT NULL,
    punto_id               UUID,
    orden                  SMALLINT     NOT NULL CHECK (orden > 0),
    enunciado              TEXT         NOT NULL CHECK (char_length(enunciado) BETWEEN 1 AND 1000),
    tipo                   TEXT         NOT NULL CHECK (tipo IN ('si_no', 'seleccion_unica',
                                                                 'seleccion_multiple', 'eleccion')),
    tipo_voto              TEXT         NOT NULL CHECK (tipo_voto IN ('nominal', 'reservado')),
    base_computo           TEXT         NOT NULL CHECK (base_computo IN ('coeficiente', 'unidades')),
    regla_mayoria          TEXT         NOT NULL CHECK (regla_mayoria IN ('simple', 'calificada_70',
                                                                          'personalizada')),
    umbral_porcentaje      NUMERIC(7,4) CHECK (umbral_porcentaje > 0 AND umbral_porcentaje <= 100),
    umbral_sobre           TEXT         CHECK (umbral_sobre IN ('presentes', 'total')),
    max_selecciones        SMALLINT     NOT NULL DEFAULT 1 CHECK (max_selecciones BETWEEN 1 AND 50),
    -- Elección por cociente electoral: número de cargos a proveer.
    cupos                  SMALLINT     CHECK (cupos BETWEEN 1 AND 50),
    -- Cargo que se elige (crea las asignaciones de rol con origen `asamblea`).
    cargo                  TEXT         CHECK (cargo IN ('presidente_asamblea', 'secretario_asamblea',
                                                         'comision_verificadora', 'consejero',
                                                         'revisor_fiscal', 'comite_convivencia')),
    estado                 TEXT         NOT NULL DEFAULT 'borrador'
                           CHECK (estado IN ('borrador', 'abierta', 'cerrada', 'anulada')),
    temporizador_segundos  INTEGER      CHECK (temporizador_segundos BETWEEN 10 AND 86400),
    cierra_en              TIMESTAMPTZ,
    abierta_en             TIMESTAMPTZ,
    cerrada_en             TIMESTAMPTZ,
    -- Resultado calculado al cerrar (por coeficiente y por unidades); no es fuente de verdad.
    resultado              JSONB,
    roles_aplicados_en     TIMESTAMPTZ,
    creado_en              TIMESTAMPTZ  NOT NULL DEFAULT now(),
    UNIQUE (reunion_id, orden),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, reunion_id) REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, punto_id)   REFERENCES puntos_orden_dia (tenant_id, id),
    CHECK ((regla_mayoria = 'personalizada') = (umbral_porcentaje IS NOT NULL AND umbral_sobre IS NOT NULL)),
    CHECK ((tipo = 'eleccion') = (cupos IS NOT NULL)),
    CHECK (tipo = 'seleccion_multiple' OR max_selecciones = 1),
    CHECK (cerrada_en IS NULL OR (abierta_en IS NOT NULL AND cerrada_en >= abierta_en))
);
CREATE INDEX ix_preguntas_abiertas ON preguntas (tenant_id, reunion_id) WHERE estado = 'abierta';
CREATE INDEX ix_preguntas_por_cerrar ON preguntas (cierra_en) WHERE estado = 'abierta';

CREATE TABLE opciones (
    id           UUID     PRIMARY KEY,
    tenant_id    UUID     NOT NULL REFERENCES copropiedades (id),
    pregunta_id  UUID     NOT NULL,
    orden        SMALLINT NOT NULL CHECK (orden > 0),
    texto        TEXT     NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 300),
    -- Voto en blanco: cuenta para el cociente electoral pero no gana cupos.
    en_blanco    BOOLEAN  NOT NULL DEFAULT FALSE,
    UNIQUE (pregunta_id, orden),
    UNIQUE (tenant_id, pregunta_id, id),
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);

-- Integrantes de una plancha o candidato (elecciones de cargos).
CREATE TABLE candidatos (
    tenant_id    UUID     NOT NULL REFERENCES copropiedades (id),
    pregunta_id  UUID     NOT NULL,
    opcion_id    UUID     NOT NULL,
    orden        SMALLINT NOT NULL CHECK (orden > 0),
    persona_id   UUID     NOT NULL,
    PRIMARY KEY (opcion_id, persona_id),
    UNIQUE (opcion_id, orden),
    FOREIGN KEY (tenant_id, pregunta_id, opcion_id) REFERENCES opciones (tenant_id, pregunta_id, id),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);

-- Bienes con derecho a voto en la pregunta: presentes al abrirla (o que ingresan mientras está
-- abierta) y sin restricción por mora. Es la base de la decisión.
CREATE TABLE pregunta_habilitados (
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id      UUID          NOT NULL,
    bien_privado_id  UUID          NOT NULL,
    coeficiente      NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    desde            TIMESTAMPTZ   NOT NULL,
    PRIMARY KEY (pregunta_id, bien_privado_id),
    FOREIGN KEY (tenant_id, pregunta_id)     REFERENCES preguntas (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);

-- Foto del quórum en un momento (instalación, apertura y cierre de cada pregunta).
CREATE TABLE fotos_quorum (
    id                    UUID          PRIMARY KEY,
    tenant_id             UUID          NOT NULL REFERENCES copropiedades (id),
    reunion_id            UUID          NOT NULL,
    pregunta_id           UUID,
    momento               TEXT          NOT NULL CHECK (momento IN ('instalacion', 'apertura',
                                                                     'cierre', 'manual')),
    coeficiente_presente  NUMERIC(11,8) NOT NULL CHECK (coeficiente_presente >= 0),
    unidades_presentes    INTEGER       NOT NULL CHECK (unidades_presentes >= 0),
    coeficiente_total     NUMERIC(11,8) NOT NULL CHECK (coeficiente_total > 0),
    unidades_total        INTEGER       NOT NULL CHECK (unidades_total > 0),
    asistentes            INTEGER       NOT NULL CHECK (asistentes >= 0),
    tomada_en             TIMESTAMPTZ   NOT NULL,
    FOREIGN KEY (tenant_id, reunion_id)  REFERENCES reuniones (tenant_id, id),
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);
CREATE INDEX ix_fotos_quorum ON fotos_quorum (tenant_id, reunion_id, tomada_en);

-- ---------------------------------------------------------------------
-- Participaciones: QUIÉN votó por cada bien, cuándo, por qué canal y con qué peso (acta).
-- ---------------------------------------------------------------------
CREATE TABLE participaciones (
    id               UUID          PRIMARY KEY,
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id      UUID          NOT NULL,
    bien_privado_id  UUID          NOT NULL,
    asistente_id     UUID          NOT NULL,
    coeficiente      NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    canal            TEXT          NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial', 'rotulo')),
    emitido_por      UUID          NOT NULL,       -- persona del asistente
    apoderado_id     UUID,                         -- = emitido_por cuando representa por poder
    -- Cuenta del operador o del puesto de escaneo que registró el voto.
    registrado_por   UUID,
    idempotency_key  TEXT          NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    wamid            TEXT,
    registrado_en    TIMESTAMPTZ   NOT NULL,
    CONSTRAINT uq_participacion_bien UNIQUE (pregunta_id, bien_privado_id),
    UNIQUE (tenant_id, id),
    CHECK ((canal = 'whatsapp') = (wamid IS NOT NULL)),
    FOREIGN KEY (tenant_id, pregunta_id)     REFERENCES preguntas (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, asistente_id)    REFERENCES asistentes (tenant_id, id),
    FOREIGN KEY (tenant_id, emitido_por)     REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, apoderado_id)    REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_participaciones_reintento
    ON participaciones (pregunta_id, asistente_id, idempotency_key);

-- Solo preguntas abiertas y bienes habilitados. FOR SHARE bloquea un cierre concurrente hasta
-- el COMMIT. El adaptador busca antes el reintento por (pregunta, asistente, clave) para
-- devolver el resultado original.
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
    IF NOT EXISTS (SELECT 1 FROM pregunta_habilitados h
                   WHERE h.pregunta_id = NEW.pregunta_id
                     AND h.bien_privado_id = NEW.bien_privado_id
                     AND h.coeficiente = NEW.coeficiente) THEN
        RAISE EXCEPTION 'el bien privado no está habilitado para votar esta pregunta'
            USING ERRCODE = 'V0005';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_participaciones_validar
    BEFORE INSERT ON participaciones
    FOR EACH ROW EXECUTE FUNCTION validar_participacion();

-- ---------------------------------------------------------------------
-- Votos: QUÉ se votó (una fila por opción elegida). Sin columna de hora. En `nominal` y en
-- `reservado` se guarda el vínculo con el bien; el detalle reservado solo lo consulta quien
-- tiene `asambleas.votos.ver_detalle` (y cada consulta se audita).
-- ---------------------------------------------------------------------
CREATE TABLE votos (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    pregunta_id       UUID          NOT NULL,
    opcion_id         UUID          NOT NULL,
    coeficiente       NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    bien_privado_id   UUID          NOT NULL,
    participacion_id  UUID          NOT NULL,
    CONSTRAINT ck_voto_uuid_v4 CHECK (substring(id::text, 15, 1) = '4'),
    UNIQUE (participacion_id, opcion_id),
    FOREIGN KEY (tenant_id, pregunta_id, opcion_id) REFERENCES opciones (tenant_id, pregunta_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)  REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, participacion_id) REFERENCES participaciones (tenant_id, id)
);
CREATE INDEX ix_votos_conteo ON votos (pregunta_id, opcion_id) INCLUDE (coeficiente);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['participaciones', 'votos', 'fotos_quorum', 'envios_convocatoria'] LOOP
        EXECUTE format(
            'CREATE TRIGGER trg_%s_solo_insercion BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t, t);
        EXECUTE format(
            'CREATE TRIGGER trg_%s_sin_truncate BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t, t);
    END LOOP;
END
$$;

-- ---------------------------------------------------------------------
-- Outbox: mensajes de una asamblea (costo de WhatsApp por asamblea)
-- ---------------------------------------------------------------------
ALTER TABLE outbox ADD COLUMN reunion_id UUID;
CREATE INDEX ix_outbox_reunion ON outbox (tenant_id, reunion_id) WHERE reunion_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Plataforma: líneas de WhatsApp, acuses de entrega y tarifas
-- ---------------------------------------------------------------------
ALTER TABLE lineas_whatsapp
    ADD COLUMN estado TEXT NOT NULL DEFAULT 'activa'
        CHECK (estado IN ('activa', 'degradada', 'pausada')),
    ADD COLUMN calidad TEXT CHECK (calidad IN ('verde', 'amarilla', 'roja')),
    -- Mensajes de servicio gratuitos al mes por número (Meta, desde el 1-oct-2026).
    ADD COLUMN franquicia_servicio_mes INTEGER NOT NULL DEFAULT 1000
        CHECK (franquicia_servicio_mes >= 0),
    ADD COLUMN actualizado_en TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE lineas_whatsapp SET estado = 'pausada' WHERE NOT activa;
ALTER TABLE lineas_whatsapp DROP COLUMN activa;

-- Acuses de entrega de WhatsApp (`statuses` del webhook). Un `wamid` pasa por varios estados.
CREATE TABLE whatsapp_estados (
    wamid          TEXT        NOT NULL CHECK (char_length(wamid) BETWEEN 1 AND 200),
    linea          TEXT        NOT NULL,
    estado         TEXT        NOT NULL CHECK (estado IN ('sent', 'delivered', 'read', 'failed')),
    categoria      TEXT        CHECK (char_length(categoria) <= 40),
    facturable     BOOLEAN,
    error_codigo   INTEGER,
    ocurrido_en    TIMESTAMPTZ NOT NULL,
    recibido_en    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (wamid, estado)
);
CREATE INDEX ix_whatsapp_estados_linea ON whatsapp_estados (linea, ocurrido_en);

-- Tarifas de Meta por país y categoría (cambian cada trimestre: se editan, no se programan).
CREATE TABLE tarifas_whatsapp (
    id             UUID          PRIMARY KEY,
    pais           TEXT          NOT NULL CHECK (pais ~ '^[A-Z]{2}$'),
    categoria      TEXT          NOT NULL CHECK (categoria IN ('utilidad', 'servicio', 'marketing',
                                                               'autenticacion')),
    moneda         TEXT          NOT NULL CHECK (moneda ~ '^[A-Z]{3}$'),
    valor          NUMERIC(12,6) NOT NULL CHECK (valor >= 0),
    vigente_desde  DATE          NOT NULL,
    creado_en      TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (pais, categoria, moneda, vigente_desde)
);
INSERT INTO tarifas_whatsapp (id, pais, categoria, moneda, valor, vigente_desde) VALUES
    (gen_random_uuid(), 'CO', 'utilidad', 'COP', 2.95, '2026-10-01'),
    (gen_random_uuid(), 'CO', 'servicio', 'COP', 2.95, '2026-10-01'),
    (gen_random_uuid(), 'CO', 'utilidad', 'USD', 0.0008, '2026-10-01'),
    (gen_random_uuid(), 'CO', 'servicio', 'USD', 0.0008, '2026-10-01');

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER (cruzan ámbitos; devuelven lo mínimo)
-- ---------------------------------------------------------------------
-- Línea para iniciar una conversación (plantillas de asambleas y avisos de tickets): la activa
-- con más franquicia libre de **mensajes de servicio** este mes (Meta regala 1 000 por número;
-- las plantillas se pagan aparte y no la consumen) y, a igualdad, la de menos envíos en total
-- (reparte la carga cuando todas agotaron la franquicia). Cuenta lo encolado aunque no haya
-- salido aún, para repartir bien dentro de una misma convocatoria; no cuenta los fallidos.
CREATE FUNCTION elegir_linea_whatsapp()
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT l.phone_number_id
    FROM lineas_whatsapp l
    LEFT JOIN LATERAL (
        SELECT count(*) FILTER (WHERE o.payload -> 'contenido' ->> 'tipo' IS DISTINCT FROM 'plantilla')
                   AS servicio,
               count(*) AS enviados
        FROM outbox o
        WHERE o.canal = 'whatsapp' AND o.linea = l.phone_number_id AND o.estado <> 'fallido'
          AND o.creado_en >= date_trunc('month', now())
    ) m ON TRUE
    WHERE l.estado = 'activa'
    ORDER BY greatest(l.franquicia_servicio_mes - m.servicio, 0) DESC, m.enviados, l.creado_en, l.id
    LIMIT 1
$$;

-- Los avisos de tickets (fuera de una conversación) rotan igual que las asambleas.
CREATE OR REPLACE FUNCTION linea_whatsapp_predeterminada()
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT elegir_linea_whatsapp()
$$;

-- Asistencias activas de un teléfono (cualquier PH): reuniones en registro, instaladas o
-- suspendidas (una consulta por escrito queda instalada hasta su plazo), de copropiedades con
-- el módulo de asambleas.
CREATE FUNCTION asistencias_por_telefono(p_telefono TEXT)
RETURNS TABLE (tenant_id UUID, copropiedad TEXT, reunion_id UUID, titulo TEXT, asistente_id UUID,
               linea TEXT, confirmado BOOLEAN)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT a.tenant_id, c.nombre, r.id, r.titulo, a.id, a.linea_whatsapp,
           a.whatsapp_confirmado_en IS NOT NULL
    FROM asistentes a
    JOIN reuniones r     ON r.tenant_id = a.tenant_id AND r.id = a.reunion_id
    JOIN copropiedades c ON c.id = a.tenant_id
    WHERE a.telefono = p_telefono
      AND c.estado = 'activa'
      AND r.estado IN ('en_registro', 'instalada', 'suspendida')
      AND (r.plazo_respuesta IS NULL OR r.plazo_respuesta > now())
      AND EXISTS (SELECT 1 FROM modulos_habilitados m
                  WHERE m.tenant_id = c.id AND m.modulo = 'asambleas'
                    AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now()))
    ORDER BY r.fecha_programada, r.id
$$;

-- Copropiedades con preguntas cuyo temporizador venció o consultas escritas con el plazo
-- cumplido (el worker las cierra). Solo devuelve ids de tenant.
CREATE FUNCTION copropiedades_con_asambleas_pendientes()
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT p.tenant_id FROM preguntas p WHERE p.estado = 'abierta' AND p.cierra_en <= now()
    UNION
    SELECT r.tenant_id FROM reuniones r
    WHERE r.modalidad = 'escrita' AND r.estado = 'instalada' AND r.plazo_respuesta <= now()
$$;

-- Acuse de entrega de un mensaje de WhatsApp (el gateway no abre transacción de plataforma
-- para esto). Idempotente: Meta reintenta.
CREATE FUNCTION registrar_estado_whatsapp(p_wamid TEXT, p_linea TEXT, p_estado TEXT,
                                          p_categoria TEXT, p_facturable BOOLEAN,
                                          p_error INTEGER, p_ocurrido TIMESTAMPTZ)
RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    INSERT INTO whatsapp_estados (wamid, linea, estado, categoria, facturable, error_codigo, ocurrido_en)
    VALUES (p_wamid, p_linea, p_estado, p_categoria, p_facturable, p_error, p_ocurrido)
    ON CONFLICT (wamid, estado) DO NOTHING
$$;

-- Calidad de una línea según Meta (`phone_number_quality_update`): degradada deja de recibir
-- conversaciones nuevas; una pausada a mano no se reactiva sola.
CREATE FUNCTION actualizar_calidad_linea(p_numero TEXT, p_degradada BOOLEAN, p_calidad TEXT)
RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    UPDATE lineas_whatsapp
    SET estado = CASE WHEN estado = 'pausada' THEN 'pausada'
                      WHEN p_degradada THEN 'degradada' ELSE 'activa' END,
        calidad = coalesce(p_calidad, calidad),
        actualizado_en = now()
    WHERE regexp_replace(numero_visible, '[^0-9]', '', 'g') = regexp_replace(p_numero, '[^0-9]', '', 'g')
$$;

-- Último acuse de cada mensaje del outbox de la copropiedad de la transacción (para la mesa:
-- «no llegó: entregue rótulo»).
CREATE FUNCTION estados_entrega(p_outbox BIGINT[])
RETURNS TABLE (outbox_id BIGINT, estado TEXT, error_codigo INTEGER)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.id, coalesce(e.estado, CASE o.estado WHEN 'fallido' THEN 'failed' ELSE NULL END),
           e.error_codigo
    FROM outbox o
    LEFT JOIN LATERAL (
        SELECT w.estado, w.error_codigo FROM whatsapp_estados w
        WHERE w.wamid = o.referencia_salida
        ORDER BY array_position(ARRAY['failed', 'read', 'delivered', 'sent'], w.estado)
        LIMIT 1
    ) e ON TRUE
    WHERE o.id = ANY (p_outbox) AND o.tenant_id = app_tenant_id()
$$;

-- Mensajes de WhatsApp de una reunión por tipo (plantilla o servicio) y estado, para estimar
-- el costo de la asamblea.
CREATE FUNCTION mensajes_whatsapp_reunion(p_reunion UUID)
RETURNS TABLE (es_plantilla BOOLEAN, linea TEXT, enviados BIGINT, fallidos BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.payload -> 'contenido' ->> 'tipo' = 'plantilla', o.linea,
           count(*) FILTER (WHERE o.estado = 'enviado'),
           count(*) FILTER (WHERE o.estado = 'fallido')
    FROM outbox o
    WHERE o.tenant_id = app_tenant_id() AND o.reunion_id = p_reunion AND o.canal = 'whatsapp'
    GROUP BY 1, 2
$$;

-- Tarifas vigentes a una fecha (la tabla es de plataforma).
CREATE FUNCTION tarifas_whatsapp_vigentes(p_pais TEXT, p_moneda TEXT, p_fecha DATE)
RETURNS TABLE (categoria TEXT, valor NUMERIC)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT ON (t.categoria) t.categoria, t.valor
    FROM tarifas_whatsapp t
    WHERE t.pais = p_pais AND t.moneda = p_moneda AND t.vigente_desde <= p_fecha
    ORDER BY t.categoria, t.vigente_desde DESC
$$;

-- Mensajes de WhatsApp de este mes por línea (todas las copropiedades), separados en servicio
-- (consumen la franquicia gratuita) y plantillas (se pagan aparte).
CREATE FUNCTION uso_lineas_whatsapp()
RETURNS TABLE (linea TEXT, servicio BIGINT, plantillas BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.linea,
           count(*) FILTER (WHERE o.payload -> 'contenido' ->> 'tipo' IS DISTINCT FROM 'plantilla'),
           count(*) FILTER (WHERE o.payload -> 'contenido' ->> 'tipo' = 'plantilla')
    FROM outbox o
    WHERE o.canal = 'whatsapp' AND o.linea IS NOT NULL AND o.estado <> 'fallido'
      AND o.creado_en >= date_trunc('month', now())
    GROUP BY o.linea
$$;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'elegir_linea_whatsapp()', 'asistencias_por_telefono(TEXT)',
        'copropiedades_con_asambleas_pendientes()',
        'registrar_estado_whatsapp(TEXT, TEXT, TEXT, TEXT, BOOLEAN, INTEGER, TIMESTAMPTZ)',
        'actualizar_calidad_linea(TEXT, BOOLEAN, TEXT)', 'estados_entrega(BIGINT[])',
        'mensajes_whatsapp_reunion(UUID)', 'tarifas_whatsapp_vigentes(TEXT, TEXT, DATE)',
        'uso_lineas_whatsapp()'
    ] LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f);
        EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO dad_app', f);
    END LOOP;
END
$$;

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'reuniones', 'puntos_orden_dia', 'documentos_reunion', 'padron_reunion',
        'morosos_reunion', 'asistentes', 'poderes', 'asistente_bienes', 'asistente_presencias',
        'rotulos', 'envios_convocatoria', 'preguntas', 'opciones', 'candidatos',
        'pregunta_habilitados', 'fotos_quorum', 'participaciones', 'votos'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['whatsapp_estados', 'tarifas_whatsapp'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) WITH CHECK (app_plataforma())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT, UPDATE ON reuniones, puntos_orden_dia, asistentes, poderes,
    asistente_bienes, asistente_presencias, rotulos, preguntas TO dad_app;
GRANT SELECT, INSERT, DELETE ON documentos_reunion, padron_reunion, morosos_reunion, opciones,
    candidatos, pregunta_habilitados TO dad_app;
GRANT SELECT, INSERT ON participaciones, votos, fotos_quorum, envios_convocatoria TO dad_app;
GRANT USAGE ON SEQUENCE envios_convocatoria_id_seq TO dad_app;
GRANT DELETE ON puntos_orden_dia, preguntas TO dad_app;
GRANT SELECT, INSERT, UPDATE ON tarifas_whatsapp TO dad_app;
GRANT SELECT ON whatsapp_estados TO dad_app;
GRANT INSERT, UPDATE (estado, calidad, limite_mps, franquicia_servicio_mes, actualizado_en)
    ON lineas_whatsapp TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('asambleas.registrar', 'copropiedad'),
    ('asambleas.votos.registrar', 'copropiedad'),
    ('plataforma.whatsapp.gestionar', 'plataforma');

INSERT INTO roles (id, tenant_id, codigo, nombre, ambito, es_plantilla) VALUES
    (uuid_rol_plantilla('operador_asamblea'), NULL, 'operador_asamblea', 'Operador de asamblea',
     'copropiedad', TRUE);

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), a, p, NULL
FROM (VALUES
    ('administrador', 'copropiedad', 'asambleas.registrar'),
    ('administrador', 'copropiedad', 'asambleas.votos.registrar'),
    ('asistente_administrativo', 'copropiedad', 'asambleas.registrar'),
    ('operador_asamblea', 'copropiedad', 'asambleas.ver'),
    ('operador_asamblea', 'copropiedad', 'asambleas.registrar'),
    ('operador_asamblea', 'copropiedad', 'asambleas.votos.registrar'),
    ('superusuario', 'plataforma', 'plataforma.whatsapp.gestionar')
) AS v (r, a, p);
