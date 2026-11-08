-- Gestión administrativa (planes 07b y 07c, ADR 0009): tickets con plazos en días y horas
-- hábiles, preguntas frecuentes, chat de WhatsApp con bandeja humana, inventario de activos,
-- planes de mantenimiento, órdenes de trabajo y obligaciones periódicas.
--
-- Garantías de la base de datos:
--   1. Número de ticket y de orden único por copropiedad, serie y año (`consecutivos`).
--   2. Un mensaje de WhatsApp (mismo `wamid`) nunca crea dos tickets: UNIQUE (tenant_id,
--      idempotency_key) y `mensajes_entrantes` (wamid ya procesado).
--   3. Historiales append-only: eventos y mensajes de tickets, mensajes de conversaciones,
--      movimientos de inventario y cumplimientos de obligaciones.
--   4. Un plan preventivo genera a lo sumo una orden por fecha programada.
--   5. Montos en NUMERIC.

-- ---------------------------------------------------------------------
-- Consecutivos legibles por copropiedad, serie y año (PQR-2026-00042, OT-2026-00007)
-- ---------------------------------------------------------------------
CREATE TABLE consecutivos (
    tenant_id  UUID     NOT NULL REFERENCES copropiedades (id),
    serie      TEXT     NOT NULL CHECK (serie IN ('ticket', 'orden_trabajo')),
    anio       SMALLINT NOT NULL CHECK (anio BETWEEN 2000 AND 2999),
    ultimo     INTEGER  NOT NULL CHECK (ultimo > 0),
    PRIMARY KEY (tenant_id, serie, anio)
);

-- ---------------------------------------------------------------------
-- Configuración de tickets: jornada hábil, reapertura, categorías y plazos
-- ---------------------------------------------------------------------
CREATE TABLE configuracion_tickets (
    tenant_id        UUID        PRIMARY KEY REFERENCES copropiedades (id),
    jornada_inicio   TIME        NOT NULL,
    jornada_fin      TIME        NOT NULL CHECK (jornada_fin > jornada_inicio),
    incluye_sabado   BOOLEAN     NOT NULL,
    dias_reapertura  SMALLINT    NOT NULL CHECK (dias_reapertura BETWEEN 1 AND 60),
    actualizado_en   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Reglas propias de la PH; las que falten salen de domain::ticket::plazo_predeterminado.
CREATE TABLE plazos_ticket (
    tenant_id  UUID     NOT NULL REFERENCES copropiedades (id),
    tipo       TEXT     NOT NULL CHECK (tipo IN (
                   'peticion', 'queja', 'reclamo', 'sugerencia', 'felicitacion',
                   'solicitud_mantenimiento', 'solicitud_administrativa', 'solicitud_documentos',
                   'reporte_convivencia')),
    prioridad  TEXT     CHECK (prioridad IN ('urgente', 'alta', 'media', 'baja')),
    cantidad   INTEGER  NOT NULL CHECK (cantidad BETWEEN 1 AND 8760),
    unidad     TEXT     NOT NULL CHECK (unidad IN ('horas', 'horas_habiles', 'dias_habiles'))
);
CREATE UNIQUE INDEX ux_plazos_ticket ON plazos_ticket (tenant_id, tipo, coalesce(prioridad, '*'));

CREATE TABLE categorias_ticket (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 60),
    activa     BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id)
);
CREATE UNIQUE INDEX ux_categorias_ticket_nombre ON categorias_ticket (tenant_id, normalizar(nombre));

-- ---------------------------------------------------------------------
-- Inventario de activos (antes que tickets: un ticket puede señalar un activo)
-- ---------------------------------------------------------------------
CREATE TABLE activos (
    id                        UUID          PRIMARY KEY,
    tenant_id                 UUID          NOT NULL REFERENCES copropiedades (id),
    codigo                    TEXT          NOT NULL CHECK (codigo ~ '^[A-Z0-9-]{1,30}$'),
    nombre                    TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    clase                     TEXT          NOT NULL CHECK (clase IN ('equipo', 'herramienta', 'mobiliario', 'dotacion')),
    marca                     TEXT          CHECK (char_length(marca) <= 60),
    modelo                    TEXT          CHECK (char_length(modelo) <= 60),
    serie                     TEXT          CHECK (char_length(serie) <= 60),
    ubicacion                 TEXT          CHECK (char_length(ubicacion) <= 200),
    agrupacion_id             UUID,
    -- Bien común donde está, o la herramienta prestable (motor de reservas).
    recurso_id                UUID,
    estado                    TEXT          NOT NULL DEFAULT 'operativo' CHECK (estado IN (
                                  'operativo', 'con_falla', 'en_reparacion', 'fuera_de_servicio',
                                  'dado_de_baja')),
    fecha_compra              DATE,
    valor                     NUMERIC(16,2) CHECK (valor >= 0),
    garantia_vence            DATE,
    proveedor_organizacion_id UUID          REFERENCES organizaciones (id),
    creado_en                 TIMESTAMPTZ   NOT NULL DEFAULT now(),
    actualizado_en            TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, codigo),
    FOREIGN KEY (tenant_id, agrupacion_id) REFERENCES agrupaciones (tenant_id, id),
    FOREIGN KEY (tenant_id, recurso_id)    REFERENCES recursos (tenant_id, id)
);
CREATE INDEX ix_activos_estado ON activos (tenant_id, clase, estado);
CREATE INDEX ix_activos_nombre ON activos USING gin (normalizar(nombre) gin_trgm_ops);
CREATE INDEX ix_activos_garantia ON activos (tenant_id, garantia_vence)
    WHERE garantia_vence IS NOT NULL AND estado <> 'dado_de_baja';

CREATE TABLE activo_archivos (
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    activo_id   UUID        NOT NULL,
    archivo_id  UUID        NOT NULL,
    tipo        TEXT        NOT NULL CHECK (tipo IN ('manual', 'foto', 'factura', 'garantia', 'otro')),
    creado_en   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (activo_id, archivo_id),
    FOREIGN KEY (tenant_id, activo_id)  REFERENCES activos (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

CREATE TABLE movimientos_inventario (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    activo_id           UUID        NOT NULL,
    tipo                TEXT        NOT NULL CHECK (tipo IN (
                            'entrada', 'salida', 'traslado', 'prestamo', 'devolucion', 'baja')),
    ubicacion_destino   TEXT        CHECK (char_length(ubicacion_destino) <= 200),
    reserva_id          UUID,
    soporte_archivo_id  UUID,
    observacion         TEXT        CHECK (char_length(observacion) <= 500),
    registrado_por      UUID,       -- cuenta
    ocurrido_en         TIMESTAMPTZ NOT NULL,
    CHECK (tipo <> 'baja' OR soporte_archivo_id IS NOT NULL),
    CHECK (tipo <> 'traslado' OR ubicacion_destino IS NOT NULL),
    FOREIGN KEY (tenant_id, activo_id)          REFERENCES activos (tenant_id, id),
    FOREIGN KEY (tenant_id, reserva_id)         REFERENCES reservas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_movimientos_activo ON movimientos_inventario (tenant_id, activo_id, ocurrido_en);

-- ---------------------------------------------------------------------
-- Tickets
-- ---------------------------------------------------------------------
CREATE TABLE tickets (
    id                        UUID        PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES copropiedades (id),
    anio                      SMALLINT    NOT NULL,
    consecutivo               INTEGER     NOT NULL CHECK (consecutivo > 0),
    tipo                      TEXT        NOT NULL CHECK (tipo IN (
                                  'peticion', 'queja', 'reclamo', 'sugerencia', 'felicitacion',
                                  'solicitud_mantenimiento', 'solicitud_administrativa',
                                  'solicitud_documentos', 'reporte_convivencia')),
    categoria_id              UUID,
    prioridad                 TEXT        NOT NULL CHECK (prioridad IN ('urgente', 'alta', 'media', 'baja')),
    canal                     TEXT        NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial', 'correo')),
    estado                    TEXT        NOT NULL DEFAULT 'abierto' CHECK (estado IN (
                                  'abierto', 'en_revision', 'asignado', 'en_progreso',
                                  'en_espera_tercero', 'en_espera_solicitante', 'resuelto',
                                  'cerrado', 'reabierto', 'rechazado', 'duplicado')),
    asunto                    TEXT        NOT NULL CHECK (char_length(asunto) BETWEEN 1 AND 120),
    descripcion               TEXT        NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 5000),
    solicitante_persona_id    UUID,
    solicitante_cuenta_id     UUID,
    bien_privado_id           UUID,
    -- Ubicación: texto libre, un bien común o un activo.
    ubicacion                 TEXT        CHECK (char_length(ubicacion) <= 200),
    recurso_id                UUID,
    activo_id                 UUID,
    asignado_cuenta_id        UUID,
    asignado_organizacion_id  UUID        REFERENCES organizaciones (id),
    plazo_cantidad            INTEGER     CHECK (plazo_cantidad > 0),
    plazo_unidad              TEXT        CHECK (plazo_unidad IN ('horas', 'horas_habiles', 'dias_habiles')),
    vence_en                  TIMESTAMPTZ,
    alerta_vencimiento_en     TIMESTAMPTZ,
    resuelto_en               TIMESTAMPTZ,
    reabrible_hasta           TIMESTAMPTZ,
    cerrado_en                TIMESTAMPTZ,
    duplicado_de              UUID,
    confidencial              BOOLEAN     NOT NULL,
    idempotency_key           TEXT        NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 200),
    calificacion              SMALLINT    CHECK (calificacion BETWEEN 1 AND 5),
    comentario_encuesta       TEXT        CHECK (char_length(comentario_encuesta) <= 1000),
    creado_en                 TIMESTAMPTZ NOT NULL,
    actualizado_en            TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, anio, consecutivo),
    UNIQUE (tenant_id, idempotency_key),
    CHECK (solicitante_persona_id IS NOT NULL OR solicitante_cuenta_id IS NOT NULL),
    CHECK ((plazo_cantidad IS NULL) = (plazo_unidad IS NULL)),
    CHECK (estado <> 'duplicado' OR duplicado_de IS NOT NULL),
    CHECK (asignado_cuenta_id IS NULL OR asignado_organizacion_id IS NULL),
    FOREIGN KEY (tenant_id, categoria_id)           REFERENCES categorias_ticket (tenant_id, id),
    FOREIGN KEY (tenant_id, solicitante_persona_id) REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)        REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, recurso_id)             REFERENCES recursos (tenant_id, id),
    FOREIGN KEY (tenant_id, activo_id)              REFERENCES activos (tenant_id, id),
    FOREIGN KEY (tenant_id, duplicado_de)           REFERENCES tickets (tenant_id, id)
);
CREATE INDEX ix_tickets_estado ON tickets (tenant_id, estado, creado_en);
CREATE INDEX ix_tickets_solicitante ON tickets (tenant_id, solicitante_cuenta_id, creado_en)
    WHERE solicitante_cuenta_id IS NOT NULL;
CREATE INDEX ix_tickets_persona ON tickets (tenant_id, solicitante_persona_id, creado_en)
    WHERE solicitante_persona_id IS NOT NULL;
CREATE INDEX ix_tickets_asignado ON tickets (tenant_id, asignado_cuenta_id)
    WHERE asignado_cuenta_id IS NOT NULL;
CREATE INDEX ix_tickets_organizacion ON tickets (tenant_id, asignado_organizacion_id)
    WHERE asignado_organizacion_id IS NOT NULL;
CREATE INDEX ix_tickets_vencimiento ON tickets (vence_en)
    WHERE vence_en IS NOT NULL AND alerta_vencimiento_en IS NULL;
CREATE INDEX ix_tickets_reabrible ON tickets (reabrible_hasta) WHERE estado = 'resuelto';
COMMENT ON COLUMN tickets.descripcion IS 'dato_personal: relato del solicitante; puede nombrar a terceros';
COMMENT ON COLUMN tickets.comentario_encuesta IS 'dato_personal: opinión del solicitante';

CREATE TABLE ticket_mensajes (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    ticket_id        UUID        NOT NULL,
    autor_cuenta_id  UUID,
    autor_persona_id UUID,
    canal            TEXT        NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial', 'correo')),
    -- Nota interna: el solicitante no la ve.
    interno          BOOLEAN     NOT NULL,
    texto            TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 5000),
    creado_en        TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, ticket_id)        REFERENCES tickets (tenant_id, id),
    FOREIGN KEY (tenant_id, autor_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_ticket_mensajes ON ticket_mensajes (tenant_id, ticket_id, creado_en);
COMMENT ON COLUMN ticket_mensajes.texto IS 'dato_personal: conversación con el solicitante';

CREATE TABLE ticket_adjuntos (
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    ticket_id   UUID        NOT NULL,
    mensaje_id  UUID,
    archivo_id  UUID        NOT NULL,
    interno     BOOLEAN     NOT NULL,
    creado_en   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (ticket_id, archivo_id),
    FOREIGN KEY (tenant_id, ticket_id)  REFERENCES tickets (tenant_id, id),
    FOREIGN KEY (tenant_id, mensaje_id) REFERENCES ticket_mensajes (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

CREATE TABLE ticket_eventos (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    ticket_id        UUID        NOT NULL,
    desde            TEXT,       -- NULL = radicación
    hacia            TEXT        NOT NULL,
    actor_cuenta_id  UUID,
    motivo           TEXT        CHECK (char_length(motivo) <= 500),
    ocurrido_en      TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, ticket_id) REFERENCES tickets (tenant_id, id)
);
CREATE INDEX ix_ticket_eventos ON ticket_eventos (tenant_id, ticket_id, ocurrido_en);

-- ---------------------------------------------------------------------
-- Preguntas frecuentes del chat
-- ---------------------------------------------------------------------
CREATE TABLE preguntas_frecuentes (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    pregunta         TEXT        NOT NULL CHECK (char_length(pregunta) BETWEEN 1 AND 72),
    respuesta        TEXT        NOT NULL CHECK (char_length(respuesta) BETWEEN 1 AND 4000),
    orden            SMALLINT    NOT NULL DEFAULT 0,
    activa           BOOLEAN     NOT NULL DEFAULT TRUE,
    -- Reglamento, manual de convivencia u otro documento que el chat puede enlazar.
    archivo_id       UUID,
    actualizado_en   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_preguntas_frecuentes ON preguntas_frecuentes (tenant_id, activa, orden);

-- ---------------------------------------------------------------------
-- Conversaciones escaladas a la administración (bandeja humana)
-- ---------------------------------------------------------------------
CREATE TABLE conversaciones (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    linea               TEXT        NOT NULL,   -- phone_number_id de la línea
    telefono            TEXT        NOT NULL CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    persona_id          UUID,
    estado              TEXT        NOT NULL DEFAULT 'abierta' CHECK (estado IN ('abierta', 'cerrada')),
    atendida_por        UUID,       -- cuenta
    abierta_en          TIMESTAMPTZ NOT NULL,
    ultimo_entrante_en  TIMESTAMPTZ NOT NULL,
    cerrada_en          TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'cerrada') = (cerrada_en IS NOT NULL)),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE UNIQUE INDEX ux_conversacion_abierta ON conversaciones (tenant_id, linea, telefono)
    WHERE estado = 'abierta';
CREATE INDEX ix_conversaciones_estado ON conversaciones (tenant_id, estado, ultimo_entrante_en);
COMMENT ON COLUMN conversaciones.telefono IS 'dato_personal: teléfono de WhatsApp';

CREATE TABLE conversacion_mensajes (
    id                UUID        PRIMARY KEY,
    tenant_id         UUID        NOT NULL REFERENCES copropiedades (id),
    conversacion_id   UUID        NOT NULL,
    direccion         TEXT        NOT NULL CHECK (direccion IN ('entrante', 'saliente')),
    texto             TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 4096),
    autor_cuenta_id   UUID,
    wamid             TEXT,
    creado_en         TIMESTAMPTZ NOT NULL,
    CHECK ((direccion = 'entrante') = (wamid IS NOT NULL)),
    FOREIGN KEY (tenant_id, conversacion_id) REFERENCES conversaciones (tenant_id, id)
);
CREATE UNIQUE INDEX ux_conversacion_wamid ON conversacion_mensajes (wamid) WHERE wamid IS NOT NULL;
CREATE INDEX ix_conversacion_mensajes ON conversacion_mensajes (tenant_id, conversacion_id, creado_en);
COMMENT ON COLUMN conversacion_mensajes.texto IS 'dato_personal: contenido de la conversación';

-- ---------------------------------------------------------------------
-- Plataforma: sesiones del chat y mensajes entrantes ya procesados (sin tenant)
-- ---------------------------------------------------------------------
-- Sesión por (línea, teléfono): paso del menú, PH elegida y borrador. No es fuente de verdad.
CREATE TABLE sesiones_chat (
    linea               TEXT        NOT NULL,
    telefono            TEXT        NOT NULL CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    estado              JSONB       NOT NULL,
    ultimo_entrante_en  TIMESTAMPTZ NOT NULL,
    expira_en           TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (linea, telefono)
);
CREATE INDEX ix_sesiones_chat_expira ON sesiones_chat (expira_en);
COMMENT ON COLUMN sesiones_chat.telefono IS 'dato_personal: teléfono de WhatsApp';
COMMENT ON COLUMN sesiones_chat.estado IS 'dato_personal: borrador de la conversación';

-- Meta reintenta los webhooks: un `wamid` se procesa una sola vez.
CREATE TABLE mensajes_entrantes (
    wamid        TEXT        PRIMARY KEY CHECK (char_length(wamid) BETWEEN 1 AND 200),
    recibido_en  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------
-- Mantenimiento: planes preventivos, órdenes de trabajo y obligaciones periódicas
-- ---------------------------------------------------------------------
CREATE TABLE planes_mantenimiento (
    id                        UUID        PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES copropiedades (id),
    activo_id                 UUID,
    ubicacion                 TEXT        CHECK (char_length(ubicacion) <= 200),
    actividad                 TEXT        NOT NULL CHECK (char_length(actividad) BETWEEN 1 AND 200),
    frecuencia                TEXT        NOT NULL CHECK (frecuencia IN ('dias', 'semanas', 'meses')),
    intervalo                 SMALLINT    NOT NULL CHECK (intervalo BETWEEN 1 AND 366),
    inicio                    DATE        NOT NULL,
    -- Días antes de la fecha en que se genera la orden.
    anticipacion_dias         SMALLINT    NOT NULL DEFAULT 7 CHECK (anticipacion_dias BETWEEN 0 AND 90),
    responsable_cuenta_id     UUID,
    responsable_organizacion_id UUID      REFERENCES organizaciones (id),
    duracion_estimada_min     INTEGER     CHECK (duracion_estimada_min > 0),
    checklist                 JSONB       NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(checklist) = 'array'),
    activo                    BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    actualizado_en            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    CHECK (activo_id IS NOT NULL OR ubicacion IS NOT NULL),
    CHECK (responsable_cuenta_id IS NULL OR responsable_organizacion_id IS NULL),
    FOREIGN KEY (tenant_id, activo_id) REFERENCES activos (tenant_id, id)
);
CREATE INDEX ix_planes_mantenimiento ON planes_mantenimiento (tenant_id, activo) WHERE activo;

CREATE TABLE ordenes_trabajo (
    id                        UUID          PRIMARY KEY,
    tenant_id                 UUID          NOT NULL REFERENCES copropiedades (id),
    anio                      SMALLINT      NOT NULL,
    consecutivo               INTEGER       NOT NULL CHECK (consecutivo > 0),
    origen                    TEXT          NOT NULL CHECK (origen IN ('plan_preventivo', 'ticket', 'inspeccion')),
    tipo                      TEXT          NOT NULL CHECK (tipo IN ('preventivo', 'correctivo', 'urgente')),
    plan_id                   UUID,
    ticket_id                 UUID,
    activo_id                 UUID,
    ubicacion                 TEXT          CHECK (char_length(ubicacion) <= 200),
    descripcion               TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 2000),
    asignado_cuenta_id        UUID,
    asignado_organizacion_id  UUID          REFERENCES organizaciones (id),
    programada_para           DATE          NOT NULL,
    estado                    TEXT          NOT NULL DEFAULT 'programada' CHECK (estado IN (
                                  'programada', 'en_ejecucion', 'ejecutada', 'verificada', 'cancelada')),
    checklist                 JSONB         NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(checklist) = 'array'),
    costo_mano_obra           NUMERIC(14,2) NOT NULL DEFAULT 0 CHECK (costo_mano_obra >= 0),
    costo_repuestos           NUMERIC(14,2) NOT NULL DEFAULT 0 CHECK (costo_repuestos >= 0),
    observaciones             TEXT          CHECK (char_length(observaciones) <= 2000),
    recibido_por              TEXT          CHECK (char_length(recibido_por) <= 120),
    firma_archivo_id          UUID,
    iniciada_en               TIMESTAMPTZ,
    ejecutada_en              TIMESTAMPTZ,
    verificada_en             TIMESTAMPTZ,
    verificada_por            UUID,
    creado_en                 TIMESTAMPTZ   NOT NULL,
    actualizado_en            TIMESTAMPTZ   NOT NULL,
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, anio, consecutivo),
    CHECK ((origen = 'plan_preventivo') = (plan_id IS NOT NULL)),
    CHECK (origen <> 'ticket' OR ticket_id IS NOT NULL),
    CHECK (asignado_cuenta_id IS NULL OR asignado_organizacion_id IS NULL),
    FOREIGN KEY (tenant_id, plan_id)          REFERENCES planes_mantenimiento (tenant_id, id),
    FOREIGN KEY (tenant_id, ticket_id)        REFERENCES tickets (tenant_id, id),
    FOREIGN KEY (tenant_id, activo_id)        REFERENCES activos (tenant_id, id),
    FOREIGN KEY (tenant_id, firma_archivo_id) REFERENCES archivos (tenant_id, id)
);
-- Invariante 4: una orden por plan y fecha programada (generación idempotente).
CREATE UNIQUE INDEX ux_orden_por_periodo ON ordenes_trabajo (tenant_id, plan_id, programada_para)
    WHERE plan_id IS NOT NULL;
CREATE INDEX ix_ordenes_estado ON ordenes_trabajo (tenant_id, estado, programada_para);
CREATE INDEX ix_ordenes_activo ON ordenes_trabajo (tenant_id, activo_id) WHERE activo_id IS NOT NULL;
CREATE INDEX ix_ordenes_ticket ON ordenes_trabajo (tenant_id, ticket_id) WHERE ticket_id IS NOT NULL;
COMMENT ON COLUMN ordenes_trabajo.recibido_por IS 'dato_personal: nombre de quien recibe el trabajo';

CREATE TABLE orden_evidencias (
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    orden_id    UUID        NOT NULL,
    archivo_id  UUID        NOT NULL,
    momento     TEXT        NOT NULL CHECK (momento IN ('antes', 'despues', 'soporte')),
    creado_en   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (orden_id, archivo_id),
    FOREIGN KEY (tenant_id, orden_id)   REFERENCES ordenes_trabajo (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

CREATE TABLE obligaciones_periodicas (
    id                        UUID        PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES copropiedades (id),
    -- Código del catálogo sugerido (domain::mantenimiento::OBLIGACIONES_SUGERIDAS) o NULL.
    codigo_sugerido           TEXT        CHECK (char_length(codigo_sugerido) <= 60),
    nombre                    TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    descripcion               TEXT        CHECK (char_length(descripcion) <= 1000),
    periodicidad_meses        SMALLINT    NOT NULL CHECK (periodicidad_meses BETWEEN 1 AND 120),
    proxima                   DATE        NOT NULL,
    dias_alerta               SMALLINT    NOT NULL DEFAULT 30 CHECK (dias_alerta BETWEEN 0 AND 365),
    activo_id                 UUID,
    responsable_organizacion_id UUID      REFERENCES organizaciones (id),
    activa                    BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    actualizado_en            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, activo_id) REFERENCES activos (tenant_id, id)
);
CREATE INDEX ix_obligaciones_proxima ON obligaciones_periodicas (tenant_id, proxima) WHERE activa;
CREATE UNIQUE INDEX ux_obligacion_sugerida ON obligaciones_periodicas (tenant_id, codigo_sugerido)
    WHERE codigo_sugerido IS NOT NULL AND activa;

CREATE TABLE cumplimientos_obligacion (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    obligacion_id       UUID        NOT NULL,
    realizada_el        DATE        NOT NULL,
    soporte_archivo_id  UUID,
    observacion         TEXT        CHECK (char_length(observacion) <= 500),
    registrado_por      UUID,
    registrado_en       TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, obligacion_id)      REFERENCES obligaciones_periodicas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_cumplimientos ON cumplimientos_obligacion (tenant_id, obligacion_id, realizada_el);

-- ---------------------------------------------------------------------
-- Historiales append-only (también por permisos: dad_app solo tiene INSERT)
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'ticket_mensajes', 'ticket_adjuntos', 'ticket_eventos', 'conversacion_mensajes',
        'movimientos_inventario', 'cumplimientos_obligacion'
    ] LOOP
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
-- Funciones SECURITY DEFINER (cruzan ámbitos; devuelven lo mínimo)
-- ---------------------------------------------------------------------
-- Vínculos vigentes de un teléfono (titular o residente) en las copropiedades activas que tienen
-- contratada la gestión administrativa. El chat de WhatsApp resuelve así a quién atiende:
-- cualquier línea atiende cualquier PH (decisión del 2026-11-07).
CREATE FUNCTION vinculos_por_telefono(p_telefono TEXT)
RETURNS TABLE (tenant_id UUID, copropiedad TEXT, persona_id UUID, cuenta_id UUID,
               bien_privado_id UUID, codigo TEXT, vinculo TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH vinculos AS (
        SELECT t.tenant_id, t.persona_id, t.bien_privado_id, 'propietario' AS vinculo
        FROM titularidades t
        WHERE t.vigente_desde <= now() AND (t.vigente_hasta IS NULL OR t.vigente_hasta > now())
        UNION
        SELECT r.tenant_id, r.persona_id, r.bien_privado_id, 'residente'
        FROM residencias r
        WHERE r.vigente_desde <= now() AND (r.vigente_hasta IS NULL OR r.vigente_hasta > now())
    )
    SELECT DISTINCT ON (c.id, p.id, b.id)
           c.id, c.nombre, p.id, p.cuenta_id, b.id, b.codigo, v.vinculo
    FROM personas p
    JOIN vinculos v       ON v.tenant_id = p.tenant_id AND v.persona_id = p.id
    JOIN bienes_privados b ON b.tenant_id = v.tenant_id AND b.id = v.bien_privado_id
    JOIN copropiedades c  ON c.id = p.tenant_id
    WHERE p.telefono = p_telefono
      AND c.estado = 'activa'
      AND EXISTS (SELECT 1 FROM modulos_habilitados m
                  WHERE m.tenant_id = c.id AND m.modulo = 'gestion_administrativa'
                    AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now()))
    ORDER BY c.id, p.id, b.id, v.vinculo
$$;

-- Cuentas con un permiso vigente en la copropiedad de la transacción, con su correo y, si se
-- registraron como personas de la PH, su teléfono. Para avisos internos (urgencias, vencimientos).
CREATE FUNCTION destinatarios_con_permiso(p_permiso TEXT)
RETURNS TABLE (cuenta_id UUID, correo TEXT, telefono TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT ON (c.id) c.id, c.correo, p.telefono
    FROM asignaciones_rol a
    JOIN roles_permisos rp ON rp.rol_id = a.rol_id AND rp.permiso = p_permiso
    JOIN cuentas c         ON c.id = a.cuenta_id AND c.estado = 'activa'
    LEFT JOIN personas p   ON p.tenant_id = a.tenant_id AND p.cuenta_id = c.id AND p.telefono IS NOT NULL
    WHERE a.tenant_id = app_tenant_id()
      AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
    ORDER BY c.id
$$;

-- Línea de WhatsApp para avisos que no responden a una conversación (la más antigua activa).
CREATE FUNCTION linea_whatsapp_predeterminada()
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT phone_number_id FROM lineas_whatsapp WHERE activa ORDER BY creado_en, id LIMIT 1
$$;

-- Copropiedades con tareas del worker de gestión: tickets por cerrar o vencidos sin alerta,
-- planes preventivos activos u obligaciones por vencer. Solo devuelve ids de tenant.
CREATE FUNCTION copropiedades_con_gestion_pendiente()
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT t.tenant_id FROM tickets t
    WHERE (t.estado = 'resuelto' AND t.reabrible_hasta <= now())
       OR (t.vence_en <= now() AND t.alerta_vencimiento_en IS NULL
           AND t.estado NOT IN ('resuelto', 'cerrado', 'rechazado', 'duplicado', 'en_espera_solicitante'))
    UNION
    SELECT p.tenant_id FROM planes_mantenimiento p WHERE p.activo
$$;

REVOKE ALL ON FUNCTION vinculos_por_telefono(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION destinatarios_con_permiso(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION linea_whatsapp_predeterminada() FROM PUBLIC;
REVOKE ALL ON FUNCTION copropiedades_con_gestion_pendiente() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION vinculos_por_telefono(TEXT) TO dad_app;
GRANT EXECUTE ON FUNCTION destinatarios_con_permiso(TEXT) TO dad_app;
GRANT EXECUTE ON FUNCTION linea_whatsapp_predeterminada() TO dad_app;
GRANT EXECUTE ON FUNCTION copropiedades_con_gestion_pendiente() TO dad_app;

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'consecutivos', 'configuracion_tickets', 'plazos_ticket', 'categorias_ticket', 'activos',
        'activo_archivos', 'movimientos_inventario', 'tickets', 'ticket_mensajes',
        'ticket_adjuntos', 'ticket_eventos', 'preguntas_frecuentes', 'conversaciones',
        'conversacion_mensajes', 'planes_mantenimiento', 'ordenes_trabajo', 'orden_evidencias',
        'obligaciones_periodicas', 'cumplimientos_obligacion'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['sesiones_chat', 'mensajes_entrantes'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) WITH CHECK (app_plataforma())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT ON consecutivos, ticket_mensajes, ticket_adjuntos, ticket_eventos,
    conversacion_mensajes, movimientos_inventario, cumplimientos_obligacion, activo_archivos,
    orden_evidencias, mensajes_entrantes TO dad_app;
GRANT UPDATE (ultimo) ON consecutivos TO dad_app;
GRANT SELECT, INSERT, UPDATE ON configuracion_tickets, categorias_ticket, tickets,
    preguntas_frecuentes, conversaciones, activos, planes_mantenimiento, ordenes_trabajo,
    obligaciones_periodicas TO dad_app;
GRANT SELECT, INSERT, DELETE ON plazos_ticket TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON sesiones_chat TO dad_app;
GRANT DELETE ON mensajes_entrantes TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('tickets.ver_todos', 'copropiedad'),
    ('tickets.configurar', 'copropiedad'),
    ('tickets.convivencia.ver', 'copropiedad'),
    ('tickets.urgencias.recibir', 'copropiedad'),
    ('conversaciones.atender', 'copropiedad'),
    ('preguntas_frecuentes.editar', 'copropiedad'),
    ('inventario.ver', 'copropiedad'),
    ('inventario.editar', 'copropiedad'),
    ('mantenimiento.planificar', 'copropiedad'),
    ('mantenimiento.ejecutar', 'copropiedad'),
    ('mantenimiento.verificar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('administrador'), 'copropiedad', p, NULL
FROM unnest(ARRAY['tickets.ver_todos', 'tickets.configurar', 'tickets.convivencia.ver',
                  'tickets.urgencias.recibir', 'conversaciones.atender',
                  'preguntas_frecuentes.editar', 'inventario.ver', 'inventario.editar',
                  'mantenimiento.planificar', 'mantenimiento.ejecutar',
                  'mantenimiento.verificar']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM (VALUES
    ('asistente_administrativo', 'tickets.ver_todos'),
    ('asistente_administrativo', 'tickets.atender'),
    ('asistente_administrativo', 'conversaciones.atender'),
    ('asistente_administrativo', 'preguntas_frecuentes.editar'),
    ('asistente_administrativo', 'inventario.ver'),
    ('asistente_administrativo', 'inventario.editar'),
    ('asistente_administrativo', 'mantenimiento.planificar'),
    ('asistente_administrativo', 'mantenimiento.verificar'),
    ('revisor_fiscal', 'inventario.ver'),
    ('presidente_consejo', 'tickets.ver_todos'),
    ('comite_convivencia', 'tickets.convivencia.ver'),
    ('supervisor_seguridad', 'tickets.urgencias.recibir'),
    ('mantenimiento', 'tickets.urgencias.recibir'),
    ('mantenimiento', 'mantenimiento.ejecutar'),
    ('mantenimiento', 'inventario.ver'),
    ('proveedor_tecnico', 'mantenimiento.ejecutar')
) AS v (r, p);
