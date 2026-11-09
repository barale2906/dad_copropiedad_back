-- Control de acceso, visitas y correspondencia (plan 05, ADR 0011).
--
-- Garantías de la base de datos:
--   1. Historiales append-only: eventos de visitas, entregas de correspondencia, minuta y
--      eventos de acceso (estos solo se borran al vencer el plazo de retención, por función).
--   2. Una correspondencia tiene a lo sumo una entrega, con firma o foto y la identidad de quien
--      recogió (persona registrada o tercero con documento).
--   3. Ningún identificador de credencial en claro: solo su huella SHA-256. No hay dos
--      credenciales vigentes con la misma huella.
--   4. Cada cambio de credenciales, autorizaciones, residencias o titularidades sube la versión
--      de sincronización de las credenciales afectadas: el puente las retira en la siguiente
--      sincronización (invariante 8: las autorizaciones derivadas se calculan del censo).
--   5. Los avisos de acceso quedan marcados en el outbox (`concepto`) para llevar el costo de
--      WhatsApp por PH.
--   6. Los eventos de acceso se particionan por mes.

-- ---------------------------------------------------------------------
-- Configuración por copropiedad
-- ---------------------------------------------------------------------
CREATE TABLE configuracion_acceso (
    tenant_id                    UUID        PRIMARY KEY REFERENCES copropiedades (id),
    retencion_soportes_dias      SMALLINT    NOT NULL CHECK (retencion_soportes_dias BETWEEN 30 AND 3650),
    dias_avisos_correspondencia  SMALLINT    NOT NULL CHECK (dias_avisos_correspondencia BETWEEN 1 AND 60),
    silencio_inicio              TIME,
    silencio_fin                 TIME,
    retencion_eventos_dias       SMALLINT    NOT NULL CHECK (retencion_eventos_dias BETWEEN 365 AND 730),
    retencion_fotos_placas_dias  SMALLINT    NOT NULL CHECK (retencion_fotos_placas_dias BETWEEN 1 AND 90),
    actualizado_en               TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((silencio_inicio IS NULL) = (silencio_fin IS NULL)),
    CHECK (silencio_inicio IS DISTINCT FROM silencio_fin OR silencio_inicio IS NULL)
);

-- ---------------------------------------------------------------------
-- Puntos de acceso, puentes y dispositivos
-- ---------------------------------------------------------------------
CREATE TABLE puntos_acceso (
    id          UUID        PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    nombre      TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    tipo        TEXT        NOT NULL CHECK (tipo IN ('peatonal', 'vehicular', 'mixto', 'interno')),
    sentido     TEXT        NOT NULL CHECK (sentido IN ('entrada', 'salida', 'ambos')),
    recurso_id  UUID,
    activo      BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, recurso_id) REFERENCES recursos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_puntos_acceso_nombre ON puntos_acceso (tenant_id, normalizar(nombre));
CREATE INDEX ix_puntos_acceso_recurso ON puntos_acceso (tenant_id, recurso_id) WHERE recurso_id IS NOT NULL;

CREATE TABLE puentes (
    id                UUID        PRIMARY KEY,
    tenant_id         UUID        NOT NULL REFERENCES copropiedades (id),
    nombre            TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    estado            TEXT        NOT NULL CHECK (estado IN ('activo', 'retirado')),
    -- SHA-256 del token de máquina; el token solo se muestra al registrarlo o rotarlo.
    credencial_hash   TEXT        NOT NULL UNIQUE CHECK (credencial_hash ~ '^[0-9a-f]{64}$'),
    version           TEXT        CHECK (char_length(version) <= 40),
    ultimo_latido_en  TIMESTAMPTZ,
    creado_en         TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id)
);

CREATE TABLE dispositivos_acceso (
    id               UUID    PRIMARY KEY,
    tenant_id        UUID    NOT NULL REFERENCES copropiedades (id),
    puente_id        UUID    NOT NULL,
    punto_acceso_id  UUID    NOT NULL,
    tipo             TEXT    NOT NULL CHECK (tipo IN (
                         'torniquete', 'talanquera', 'puerta', 'lector_qr', 'lector_rfid',
                         'teclado_pin', 'biometrico_huella', 'biometrico_facial', 'camara_lpr')),
    fabricante       TEXT    NOT NULL CHECK (char_length(fabricante) BETWEEN 1 AND 60),
    modelo           TEXT    CHECK (char_length(modelo) <= 60),
    -- Configuración del adaptador SIN secretos (las contraseñas viven en el puente).
    config           JSONB   NOT NULL DEFAULT '{}'::jsonb,
    activo           BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, puente_id)       REFERENCES puentes (tenant_id, id),
    FOREIGN KEY (tenant_id, punto_acceso_id) REFERENCES puntos_acceso (tenant_id, id)
);
CREATE INDEX ix_dispositivos_puente ON dispositivos_acceso (tenant_id, puente_id);

-- ---------------------------------------------------------------------
-- Visitas
-- ---------------------------------------------------------------------
CREATE SEQUENCE acceso_version_seq;

CREATE TABLE visitas (
    id                       UUID        PRIMARY KEY,
    tenant_id                UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id          UUID        NOT NULL,
    origen                   TEXT        NOT NULL CHECK (origen IN ('anuncio_porteria', 'preautorizada')),
    tipo                     TEXT        NOT NULL CHECK (tipo IN (
                                 'visita', 'domicilio', 'proveedor', 'trasteo', 'servicio_domestico')),
    estado                   TEXT        NOT NULL CHECK (estado IN (
                                 'anunciada', 'requiere_llamada', 'autorizada', 'rechazada',
                                 'sin_respuesta', 'ingresada', 'finalizada', 'vencida', 'cancelada')),
    visitante_nombre         TEXT        NOT NULL CHECK (char_length(visitante_nombre) BETWEEN 1 AND 120),
    visitante_documento      TEXT        CHECK (visitante_documento ~ '^[A-Z0-9]{3,20}$'),
    placa                    TEXT        CHECK (placa ~ '^[A-Z0-9]{4,8}$'),
    observacion              TEXT        CHECK (char_length(observacion) <= 300),
    decidida_por_persona_id  UUID,
    canal_decision           TEXT        CHECK (canal_decision IN ('app', 'web', 'whatsapp', 'llamada')),
    ventana_desde            TIMESTAMPTZ,
    ventana_hasta            TIMESTAMPTZ CHECK (ventana_hasta > ventana_desde),
    usos_permitidos          SMALLINT    NOT NULL CHECK (usos_permitidos BETWEEN 1 AND 60),
    usos                     SMALLINT    NOT NULL DEFAULT 0 CHECK (usos BETWEEN 0 AND usos_permitidos),
    avisos_enviados          SMALLINT    NOT NULL DEFAULT 0 CHECK (avisos_enviados >= 0),
    siguiente_paso_en        TIMESTAMPTZ,
    registrada_por           UUID        REFERENCES cuentas (id),
    creada_en                TIMESTAMPTZ NOT NULL,
    decidida_en              TIMESTAMPTZ,
    ingreso_en               TIMESTAMPTZ,
    salida_en                TIMESTAMPTZ,
    -- Versión de sincronización (los puentes validan los QR de las pre-autorizaciones).
    version                  BIGINT      NOT NULL DEFAULT nextval('acceso_version_seq'),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)         REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, decidida_por_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_visitas_bien ON visitas (tenant_id, bien_privado_id, id DESC);
CREATE INDEX ix_visitas_estado ON visitas (tenant_id, estado, id DESC);
CREATE INDEX ix_visitas_paso ON visitas (siguiente_paso_en) WHERE siguiente_paso_en IS NOT NULL;
CREATE INDEX ix_visitas_version ON visitas (tenant_id, version) WHERE origen = 'preautorizada';

COMMENT ON COLUMN visitas.visitante_nombre IS 'dato_personal: nombre del visitante';
COMMENT ON COLUMN visitas.visitante_documento IS 'dato_personal: documento del visitante';
COMMENT ON COLUMN visitas.placa IS 'dato_personal: placa del vehículo del visitante';

CREATE TABLE visita_eventos (
    id           BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    visita_id    UUID        NOT NULL,
    tipo         TEXT        NOT NULL CHECK (tipo IN (
                     'anunciada', 'preautorizada', 'aviso_enviado', 'autorizada', 'rechazada',
                     'escalada_llamada', 'llamada', 'ingreso', 'salida', 'vencida', 'cancelada')),
    canal        TEXT        CHECK (canal IN ('app', 'web', 'whatsapp', 'llamada')),
    persona_id   UUID,
    cuenta_id    UUID        REFERENCES cuentas (id),
    nivel        SMALLINT    CHECK (nivel >= 1),
    destinatarios SMALLINT   CHECK (destinatarios >= 0),
    ocurrido_en  TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, visita_id)  REFERENCES visitas (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_visita_eventos ON visita_eventos (tenant_id, visita_id, id);

-- ---------------------------------------------------------------------
-- Correspondencia
-- ---------------------------------------------------------------------
CREATE TABLE correspondencia (
    id                        UUID        PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id           UUID        NOT NULL,
    tipo                      TEXT        NOT NULL CHECK (tipo IN ('paquete', 'sobre', 'recibo', 'otro')),
    estado                    TEXT        NOT NULL CHECK (estado IN ('recibida', 'entregada', 'devuelta')),
    remitente                 TEXT        CHECK (char_length(remitente) <= 120),
    guia                      TEXT        CHECK (char_length(guia) <= 60),
    ubicacion                 TEXT        CHECK (char_length(ubicacion) <= 60),
    foto_archivo_id           UUID,
    recibida_por              UUID        REFERENCES cuentas (id),
    recibida_en               TIMESTAMPTZ NOT NULL,
    avisos_enviados           SMALLINT    NOT NULL DEFAULT 0 CHECK (avisos_enviados >= 0),
    ultimo_aviso_en           TIMESTAMPTZ,
    siguiente_aviso_en        TIMESTAMPTZ,
    alerta_administracion_en  TIMESTAMPTZ,
    cerrada_en                TIMESTAMPTZ,
    motivo_devolucion         TEXT        CHECK (char_length(motivo_devolucion) <= 300),
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'recibida') = (cerrada_en IS NULL)),
    CHECK (estado = 'recibida' OR siguiente_aviso_en IS NULL),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, foto_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_correspondencia_bien ON correspondencia (tenant_id, bien_privado_id, id DESC);
CREATE INDEX ix_correspondencia_estado ON correspondencia (tenant_id, estado, id DESC);
CREATE INDEX ix_correspondencia_aviso ON correspondencia (siguiente_aviso_en)
    WHERE estado = 'recibida';

CREATE TABLE entregas_correspondencia (
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    correspondencia_id  UUID        NOT NULL,
    persona_id          UUID,
    tercero_nombre      TEXT        CHECK (char_length(tercero_nombre) BETWEEN 1 AND 120),
    tercero_documento   TEXT        CHECK (tercero_documento ~ '^[A-Z0-9]{3,20}$'),
    firma_archivo_id    UUID,
    foto_archivo_id     UUID,
    entregada_por       UUID        REFERENCES cuentas (id),
    entregada_en        TIMESTAMPTZ NOT NULL,
    soportes_hasta      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, correspondencia_id),
    CHECK ((persona_id IS NOT NULL) <> (tercero_nombre IS NOT NULL)),
    CHECK ((tercero_nombre IS NULL) = (tercero_documento IS NULL)),
    CHECK (firma_archivo_id IS NOT NULL OR foto_archivo_id IS NOT NULL),
    FOREIGN KEY (tenant_id, correspondencia_id) REFERENCES correspondencia (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)         REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, firma_archivo_id)   REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, foto_archivo_id)    REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_entregas_soportes ON entregas_correspondencia (soportes_hasta);

COMMENT ON COLUMN entregas_correspondencia.tercero_nombre IS 'dato_personal: nombre de quien recogió';
COMMENT ON COLUMN entregas_correspondencia.tercero_documento IS 'dato_personal: documento de quien recogió';

-- Purga de la firma y la foto al vencer `soportes_hasta` (la entrega es append-only).
CREATE TABLE purgas_soportes_entrega (
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    correspondencia_id  UUID        NOT NULL,
    purgado_en          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, correspondencia_id),
    FOREIGN KEY (tenant_id, correspondencia_id) REFERENCES correspondencia (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Minuta de portería
-- ---------------------------------------------------------------------
CREATE TABLE minuta_porteria (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    tipo             TEXT        NOT NULL CHECK (tipo IN (
                         'inicio_turno', 'entrega_turno', 'ronda', 'novedad', 'incidente', 'otro')),
    texto            TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 2000),
    punto_acceso_id  UUID,
    registrada_por   UUID        REFERENCES cuentas (id),
    ocurrida_en      TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, punto_acceso_id) REFERENCES puntos_acceso (tenant_id, id)
);
CREATE INDEX ix_minuta ON minuta_porteria (tenant_id, id DESC);
COMMENT ON COLUMN minuta_porteria.texto IS 'dato_personal: novedades del turno (pueden nombrar personas)';

CREATE TABLE minuta_adjuntos (
    tenant_id   UUID NOT NULL REFERENCES copropiedades (id),
    novedad_id  UUID NOT NULL,
    archivo_id  UUID NOT NULL,
    PRIMARY KEY (tenant_id, novedad_id, archivo_id),
    FOREIGN KEY (tenant_id, novedad_id) REFERENCES minuta_porteria (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Credenciales y autorizaciones
-- ---------------------------------------------------------------------
CREATE TABLE credenciales (
    id                     UUID        PRIMARY KEY,
    tenant_id              UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id             UUID,
    vehiculo_id            UUID,
    tipo                   TEXT        NOT NULL CHECK (tipo IN (
                               'tarjeta_rfid', 'pin', 'qr_permanente', 'qr_temporal', 'placa',
                               'biometria_ref', 'app_movil')),
    -- SHA-256(tenant:tipo:identificador normalizado). Nunca el identificador.
    huella                 TEXT        NOT NULL CHECK (huella ~ '^[0-9a-f]{64}$'),
    estado                 TEXT        NOT NULL CHECK (estado IN ('activa', 'suspendida', 'revocada', 'perdida')),
    vigente_desde          TIMESTAMPTZ NOT NULL,
    vigente_hasta          TIMESTAMPTZ CHECK (vigente_hasta > vigente_desde),
    borrado_confirmado_en  TIMESTAMPTZ,
    creada_en              TIMESTAMPTZ NOT NULL,
    version                BIGINT      NOT NULL DEFAULT nextval('acceso_version_seq'),
    UNIQUE (tenant_id, id),
    CHECK ((persona_id IS NULL) <> (vehiculo_id IS NULL)),
    CHECK (tipo <> 'biometria_ref' OR persona_id IS NOT NULL),
    FOREIGN KEY (tenant_id, persona_id)  REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, vehiculo_id) REFERENCES vehiculos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_credenciales_huella_vigente ON credenciales (tenant_id, huella)
    WHERE estado IN ('activa', 'suspendida');
CREATE INDEX ix_credenciales_persona ON credenciales (tenant_id, persona_id) WHERE persona_id IS NOT NULL;
CREATE INDEX ix_credenciales_vehiculo ON credenciales (tenant_id, vehiculo_id) WHERE vehiculo_id IS NOT NULL;
CREATE INDEX ix_credenciales_version ON credenciales (tenant_id, version);

CREATE TABLE autorizaciones_acceso (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id     UUID,
    vehiculo_id    UUID,
    -- Las derivadas (residencia, propiedad, reserva) se calculan; aquí solo las manuales.
    origen         TEXT        NOT NULL CHECK (origen IN ('manual', 'contrato', 'visita')),
    puntos         UUID[]      NOT NULL DEFAULT '{}',
    horario        JSONB,
    vigente_desde  TIMESTAMPTZ NOT NULL,
    vigente_hasta  TIMESTAMPTZ CHECK (vigente_hasta > vigente_desde),
    motivo         TEXT        CHECK (char_length(motivo) <= 300),
    revocada_en    TIMESTAMPTZ,
    creada_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    CHECK ((persona_id IS NULL) <> (vehiculo_id IS NULL)),
    FOREIGN KEY (tenant_id, persona_id)  REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, vehiculo_id) REFERENCES vehiculos (tenant_id, id)
);
CREATE INDEX ix_autorizaciones_persona ON autorizaciones_acceso (tenant_id, persona_id) WHERE persona_id IS NOT NULL;
CREATE INDEX ix_autorizaciones_vehiculo ON autorizaciones_acceso (tenant_id, vehiculo_id) WHERE vehiculo_id IS NOT NULL;

-- La versión de una credencial sube con cada cambio propio o de lo que la habilita.
CREATE FUNCTION credencial_nueva_version() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.version := nextval('acceso_version_seq');
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_credenciales_version BEFORE UPDATE ON credenciales
    FOR EACH ROW EXECUTE FUNCTION credencial_nueva_version();
CREATE TRIGGER trg_visitas_version BEFORE UPDATE ON visitas
    FOR EACH ROW WHEN (OLD.estado IS DISTINCT FROM NEW.estado OR OLD.usos IS DISTINCT FROM NEW.usos)
    EXECUTE FUNCTION credencial_nueva_version();

-- Credenciales de una persona (y de los vehículos de sus bienes) pasan a la siguiente versión.
CREATE FUNCTION tocar_credenciales_de_persona(p_tenant UUID, p_persona UUID, p_bien UUID)
RETURNS void LANGUAGE sql AS $$
    UPDATE credenciales c SET version = nextval('acceso_version_seq')
    WHERE c.tenant_id = p_tenant
      AND (c.persona_id = p_persona
           OR c.vehiculo_id IN (SELECT v.id FROM vehiculos v
                                WHERE v.tenant_id = p_tenant AND v.bien_privado_id = p_bien))
$$;

CREATE FUNCTION sincronizar_por_vinculo() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM tocar_credenciales_de_persona(NEW.tenant_id, NEW.persona_id, NEW.bien_privado_id);
    RETURN NULL;
END
$$;
CREATE TRIGGER trg_residencias_sincronizar AFTER INSERT OR UPDATE ON residencias
    FOR EACH ROW EXECUTE FUNCTION sincronizar_por_vinculo();
CREATE TRIGGER trg_titularidades_sincronizar AFTER INSERT OR UPDATE ON titularidades
    FOR EACH ROW EXECUTE FUNCTION sincronizar_por_vinculo();

CREATE FUNCTION sincronizar_por_autorizacion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE credenciales c SET version = nextval('acceso_version_seq')
    WHERE c.tenant_id = NEW.tenant_id
      AND (c.persona_id = NEW.persona_id OR c.vehiculo_id = NEW.vehiculo_id);
    RETURN NULL;
END
$$;
CREATE TRIGGER trg_autorizaciones_sincronizar AFTER INSERT OR UPDATE ON autorizaciones_acceso
    FOR EACH ROW EXECUTE FUNCTION sincronizar_por_autorizacion();

CREATE FUNCTION sincronizar_por_reserva() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.solicitante_persona_id IS NOT NULL THEN
        UPDATE credenciales c SET version = nextval('acceso_version_seq')
        WHERE c.tenant_id = NEW.tenant_id AND c.persona_id = NEW.solicitante_persona_id;
    END IF;
    RETURN NULL;
END
$$;
CREATE TRIGGER trg_reservas_sincronizar AFTER INSERT OR UPDATE OF estado, franja ON reservas
    FOR EACH ROW EXECUTE FUNCTION sincronizar_por_reserva();

-- Ventanas vigentes de un titular: manuales + derivadas (invariante 8).
--   - Persona con residencia o titularidad vigente: todos los puntos, sin horario.
--   - Vehículo vigente de un bien con residentes o titulares vigentes: todos los puntos.
--   - Reserva aprobada o en curso de un recurso con acceso automático: los puntos del recurso
--     durante la franja de la reserva.
CREATE FUNCTION ventanas_acceso(p_persona UUID, p_vehiculo UUID, p_ahora TIMESTAMPTZ)
RETURNS TABLE (puntos UUID[], vigente_desde TIMESTAMPTZ, vigente_hasta TIMESTAMPTZ, horario JSONB)
LANGUAGE sql STABLE AS $$
    SELECT a.puntos, a.vigente_desde, a.vigente_hasta, a.horario
    FROM autorizaciones_acceso a
    WHERE a.revocada_en IS NULL AND a.vigente_desde <= p_ahora
      AND (a.vigente_hasta IS NULL OR a.vigente_hasta > p_ahora)
      AND (a.persona_id = p_persona OR a.vehiculo_id = p_vehiculo)
    UNION ALL
    SELECT '{}'::uuid[], r.vigente_desde, NULL::timestamptz, NULL::jsonb
    FROM residencias r
    WHERE r.persona_id = p_persona AND r.vigente_desde <= p_ahora
      AND (r.vigente_hasta IS NULL OR r.vigente_hasta > p_ahora)
    UNION ALL
    SELECT '{}'::uuid[], t.vigente_desde, NULL::timestamptz, NULL::jsonb
    FROM titularidades t
    WHERE t.persona_id = p_persona AND t.vigente_desde <= p_ahora
      AND (t.vigente_hasta IS NULL OR t.vigente_hasta > p_ahora)
    UNION ALL
    SELECT '{}'::uuid[], v.vigente_desde, NULL::timestamptz, NULL::jsonb
    FROM vehiculos v
    WHERE v.id = p_vehiculo AND v.vigente_desde <= p_ahora
      AND (v.vigente_hasta IS NULL OR v.vigente_hasta > p_ahora)
      AND (EXISTS (SELECT 1 FROM residencias r WHERE r.tenant_id = v.tenant_id
                     AND r.bien_privado_id = v.bien_privado_id AND r.vigente_desde <= p_ahora
                     AND (r.vigente_hasta IS NULL OR r.vigente_hasta > p_ahora))
           OR EXISTS (SELECT 1 FROM titularidades t WHERE t.tenant_id = v.tenant_id
                     AND t.bien_privado_id = v.bien_privado_id AND t.vigente_desde <= p_ahora
                     AND (t.vigente_hasta IS NULL OR t.vigente_hasta > p_ahora)))
    UNION ALL
    SELECT array_agg(p.id), lower(rs.franja), upper(rs.franja), NULL::jsonb
    FROM reservas rs
    JOIN recursos rc     ON rc.tenant_id = rs.tenant_id AND rc.id = rs.recurso_id
                        AND rc.metodo_entrega = 'acceso_automatico'
    JOIN puntos_acceso p ON p.tenant_id = rs.tenant_id AND p.recurso_id = rs.recurso_id AND p.activo
    WHERE rs.solicitante_persona_id = p_persona AND rs.estado IN ('aprobada', 'en_curso')
      AND upper(rs.franja) > p_ahora
    GROUP BY rs.id, rs.franja
$$;

-- ---------------------------------------------------------------------
-- Eventos de acceso (append-only, particionados por mes)
-- ---------------------------------------------------------------------
CREATE TABLE eventos_acceso (
    id               UUID        NOT NULL,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    punto_acceso_id  UUID,
    dispositivo_id   UUID,
    puente_id        UUID,
    id_local         TEXT        CHECK (char_length(id_local) BETWEEN 1 AND 80),
    credencial_id    UUID,
    visita_id        UUID,
    sentido          TEXT        NOT NULL CHECK (sentido IN ('entrada', 'salida', 'ambos')),
    resultado        TEXT        NOT NULL CHECK (resultado IN ('permitido', 'denegado', 'forzado', 'manual')),
    motivo           TEXT        CHECK (char_length(motivo) <= 300),
    placa            TEXT        CHECK (placa ~ '^[A-Z0-9]{4,8}$'),
    registrado_por   UUID,
    ocurrido_en      TIMESTAMPTZ NOT NULL,
    registrado_en    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, ocurrido_en),
    CHECK ((puente_id IS NULL) = (id_local IS NULL)),
    FOREIGN KEY (tenant_id, punto_acceso_id) REFERENCES puntos_acceso (tenant_id, id),
    FOREIGN KEY (tenant_id, dispositivo_id)  REFERENCES dispositivos_acceso (tenant_id, id),
    FOREIGN KEY (tenant_id, puente_id)       REFERENCES puentes (tenant_id, id),
    FOREIGN KEY (tenant_id, credencial_id)   REFERENCES credenciales (tenant_id, id),
    FOREIGN KEY (tenant_id, visita_id)       REFERENCES visitas (tenant_id, id)
) PARTITION BY RANGE (ocurrido_en);
CREATE TABLE eventos_acceso_otros PARTITION OF eventos_acceso DEFAULT;
-- Idempotencia de los reportes del puente (un reintento trae el mismo id_local y la misma hora).
CREATE UNIQUE INDEX ux_eventos_acceso_local ON eventos_acceso (tenant_id, puente_id, id_local, ocurrido_en);
CREATE INDEX ix_eventos_acceso ON eventos_acceso (tenant_id, ocurrido_en DESC, id);
CREATE INDEX ix_eventos_acceso_punto ON eventos_acceso (tenant_id, punto_acceso_id, ocurrido_en DESC);
COMMENT ON COLUMN eventos_acceso.placa IS 'dato_personal: placa leída';

-- Particiones mensuales: las crea el worker con antelación (función SECURITY DEFINER).
CREATE FUNCTION crear_particiones_eventos_acceso(p_meses INTEGER)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    -- Límites en UTC: no dependen de la zona horaria de la sesión.
    inicio DATE := date_trunc('month', now() AT TIME ZONE 'UTC')::date;
    desde DATE;
    nombre TEXT;
BEGIN
    FOR i IN 0 .. greatest(0, least(p_meses, 24)) LOOP
        desde := (inicio + make_interval(months => i))::date;
        nombre := format('eventos_acceso_%s', to_char(desde, 'YYYYMM'));
        IF to_regclass('public.' || nombre) IS NULL THEN
            EXECUTE format(
                'CREATE TABLE public.%I PARTITION OF public.eventos_acceso FOR VALUES FROM (%L) TO (%L)',
                nombre, desde::timestamp AT TIME ZONE 'UTC',
                (desde + interval '1 month')::timestamp AT TIME ZONE 'UTC');
        END IF;
    END LOOP;
END
$$;
SELECT crear_particiones_eventos_acceso(3);

-- Append-only: sin UPDATE; DELETE solo de eventos con más de un año (retención, por función).
CREATE FUNCTION proteger_eventos_acceso() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND OLD.ocurrido_en < now() - interval '365 days' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'eventos_acceso es append-only (% no permitido)', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END
$$;
CREATE TRIGGER trg_eventos_acceso_solo_insercion BEFORE UPDATE OR DELETE ON eventos_acceso
    FOR EACH ROW EXECUTE FUNCTION proteger_eventos_acceso();

CREATE TABLE fotos_eventos_acceso (
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    evento_id   UUID        NOT NULL,
    archivo_id  UUID        NOT NULL,
    purgar_en   TIMESTAMPTZ NOT NULL,
    purgada_en  TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, evento_id),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_fotos_eventos_purga ON fotos_eventos_acceso (purgar_en) WHERE purgada_en IS NULL;

-- ---------------------------------------------------------------------
-- Consentimientos biométricos (Ley 1581 de 2012: dato sensible)
-- ---------------------------------------------------------------------
CREATE TABLE consentimientos_biometricos (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id          UUID        NOT NULL,
    version_texto       TEXT        NOT NULL CHECK (char_length(version_texto) BETWEEN 1 AND 40),
    soporte_archivo_id  UUID,
    otorgado_en         TIMESTAMPTZ NOT NULL,
    revocado_en         TIMESTAMPTZ CHECK (revocado_en >= otorgado_en),
    registrado_por      UUID        REFERENCES cuentas (id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)         REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_consentimiento_vigente ON consentimientos_biometricos (tenant_id, persona_id)
    WHERE revocado_en IS NULL;
COMMENT ON COLUMN consentimientos_biometricos.persona_id IS 'dato_sensible: quién autorizó tratar su biometría';

-- ---------------------------------------------------------------------
-- Dispositivos push de la app (ámbito plataforma)
-- ---------------------------------------------------------------------
CREATE TABLE dispositivos_push (
    id             UUID        PRIMARY KEY,
    cuenta_id      UUID        NOT NULL REFERENCES cuentas (id),
    plataforma     TEXT        NOT NULL CHECK (plataforma IN ('android', 'ios')),
    token          TEXT        NOT NULL UNIQUE CHECK (char_length(token) BETWEEN 20 AND 4096),
    nombre         TEXT        CHECK (char_length(nombre) <= 80),
    activo         BOOLEAN     NOT NULL DEFAULT TRUE,
    registrado_en  TIMESTAMPTZ NOT NULL,
    actualizado_en TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_dispositivos_push_cuenta ON dispositivos_push (cuenta_id) WHERE activo;
COMMENT ON COLUMN dispositivos_push.token IS 'dato_personal: token de notificaciones del teléfono';

-- ---------------------------------------------------------------------
-- Outbox: avisos de acceso (costo por PH)
-- ---------------------------------------------------------------------
ALTER TABLE outbox ADD COLUMN concepto TEXT CHECK (concepto IN ('control_acceso'));
CREATE INDEX ix_outbox_concepto ON outbox (tenant_id, concepto, creado_en) WHERE concepto IS NOT NULL;

-- ---------------------------------------------------------------------
-- Append-only
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['visita_eventos', 'entregas_correspondencia', 'minuta_porteria', 'minuta_adjuntos']
    LOOP
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
-- Copropiedades con visitas o correspondencia por escalar. Solo ids de tenant.
CREATE FUNCTION copropiedades_con_acceso_pendiente(p_ahora TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT v.tenant_id FROM visitas v WHERE v.siguiente_paso_en <= p_ahora
    UNION
    SELECT c.tenant_id FROM correspondencia c
    WHERE c.estado = 'recibida'
      AND (c.siguiente_aviso_en <= p_ahora
           OR (c.siguiente_aviso_en IS NULL AND c.alerta_administracion_en IS NULL))
$$;

-- Copropiedades activas con el módulo de control de acceso vigente (retención diaria).
CREATE FUNCTION copropiedades_con_control_acceso()
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT m.tenant_id FROM modulos_habilitados m
    JOIN copropiedades c ON c.id = m.tenant_id AND c.estado = 'activa'
    WHERE m.modulo = 'control_acceso' AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now())
$$;

-- Puente activo por la huella de su token de máquina.
CREATE FUNCTION puente_por_credencial(p_hash TEXT)
RETURNS TABLE (tenant_id UUID, puente_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT p.tenant_id, p.id FROM puentes p
    JOIN copropiedades c ON c.id = p.tenant_id AND c.estado = 'activa'
    WHERE p.credencial_hash = p_hash AND p.estado = 'activo'
$$;

-- Tokens push activos de cuentas que son personas de la copropiedad de la transacción.
CREATE FUNCTION tokens_push_de_cuentas(p_cuentas UUID[])
RETURNS TABLE (cuenta_id UUID, token TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT d.cuenta_id, d.token FROM dispositivos_push d
    WHERE d.activo AND d.cuenta_id = ANY (p_cuentas)
      AND EXISTS (SELECT 1 FROM personas p
                  WHERE p.tenant_id = app_tenant_id() AND p.cuenta_id = d.cuenta_id)
    ORDER BY d.cuenta_id, d.actualizado_en DESC
$$;

-- Push con token inválido: el mensaje queda fallido sin reintentos, el token se desactiva y,
-- si hay respaldo, se encola por WhatsApp con la misma prioridad y el mismo concepto.
CREATE FUNCTION outbox_desviar_push(p_id BIGINT, p_linea TEXT, p_destino TEXT, p_payload JSONB)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    o outbox%ROWTYPE;
BEGIN
    SELECT * INTO o FROM outbox WHERE id = p_id AND canal = 'push';
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE outbox SET estado = 'fallido', intentos = intentos + 1,
                      ultimo_error = 'token push inválido: desviado a WhatsApp'
    WHERE id = p_id;
    UPDATE dispositivos_push SET activo = FALSE, actualizado_en = now() WHERE token = o.destino;
    IF p_payload IS NOT NULL THEN
        INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto)
        VALUES (o.tenant_id, 'whatsapp', o.prioridad, p_linea, p_destino, p_payload, o.concepto);
    END IF;
END
$$;

-- Avisos de control de acceso de la copropiedad de la transacción en [desde, hasta).
CREATE FUNCTION avisos_acceso_enviados(p_desde TIMESTAMPTZ, p_hasta TIMESTAMPTZ)
RETURNS TABLE (push_enviados BIGINT, whatsapp_enviados BIGINT, fallidos BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT count(*) FILTER (WHERE o.canal = 'push' AND o.estado = 'enviado'),
           count(*) FILTER (WHERE o.canal = 'whatsapp' AND o.estado = 'enviado'),
           count(*) FILTER (WHERE o.estado = 'fallido')
    FROM outbox o
    WHERE o.tenant_id = app_tenant_id() AND o.concepto = 'control_acceso'
      AND o.creado_en >= p_desde AND o.creado_en < p_hasta
$$;

-- Borra eventos más viejos que el plazo de la copropiedad (nunca menos de un año).
CREATE FUNCTION purgar_eventos_acceso(p_dias INTEGER)
RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    n BIGINT;
BEGIN
    DELETE FROM eventos_acceso
    WHERE tenant_id = app_tenant_id()
      AND ocurrido_en < now() - make_interval(days => greatest(p_dias, 365));
    GET DIAGNOSTICS n = ROW_COUNT;
    RETURN n;
END
$$;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'copropiedades_con_acceso_pendiente(TIMESTAMPTZ)', 'copropiedades_con_control_acceso()',
        'puente_por_credencial(TEXT)', 'tokens_push_de_cuentas(UUID[])',
        'outbox_desviar_push(BIGINT, TEXT, TEXT, JSONB)',
        'avisos_acceso_enviados(TIMESTAMPTZ, TIMESTAMPTZ)', 'purgar_eventos_acceso(INTEGER)',
        'crear_particiones_eventos_acceso(INTEGER)'
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
        'configuracion_acceso', 'puntos_acceso', 'puentes', 'dispositivos_acceso', 'visitas',
        'visita_eventos', 'correspondencia', 'entregas_correspondencia', 'purgas_soportes_entrega',
        'minuta_porteria', 'minuta_adjuntos', 'credenciales', 'autorizaciones_acceso',
        'eventos_acceso', 'fotos_eventos_acceso', 'consentimientos_biometricos'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;
ALTER TABLE dispositivos_push ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON dispositivos_push
    USING (app_plataforma()) WITH CHECK (app_plataforma());

GRANT SELECT, INSERT ON visita_eventos, entregas_correspondencia, purgas_soportes_entrega,
    minuta_porteria, minuta_adjuntos, eventos_acceso, fotos_eventos_acceso TO dad_app;
GRANT UPDATE (purgada_en) ON fotos_eventos_acceso TO dad_app;
GRANT SELECT, INSERT, UPDATE ON configuracion_acceso, puntos_acceso, puentes, dispositivos_acceso,
    visitas, correspondencia, credenciales, autorizaciones_acceso, consentimientos_biometricos,
    dispositivos_push TO dad_app;
GRANT USAGE ON SEQUENCE acceso_version_seq, visita_eventos_id_seq TO dad_app;
GRANT EXECUTE ON FUNCTION ventanas_acceso(UUID, UUID, TIMESTAMPTZ) TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('acceso.correspondencia.entregar', 'copropiedad'),
    ('acceso.minuta.registrar', 'copropiedad'),
    ('acceso.minuta.ver', 'copropiedad'),
    ('acceso.configurar', 'copropiedad'),
    ('acceso.credenciales.gestionar', 'copropiedad'),
    ('acceso.biometria.enrolar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('administrador'), 'copropiedad', p, NULL
FROM unnest(ARRAY['acceso.correspondencia.entregar', 'acceso.minuta.registrar',
                  'acceso.minuta.ver', 'acceso.configurar', 'acceso.credenciales.gestionar',
                  'acceso.biometria.enrolar']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM (VALUES
    ('vigilante', 'acceso.correspondencia.entregar'),
    ('vigilante', 'acceso.minuta.registrar'),
    ('vigilante', 'acceso.minuta.ver'),
    ('supervisor_seguridad', 'acceso.correspondencia.entregar'),
    ('supervisor_seguridad', 'acceso.minuta.registrar'),
    ('supervisor_seguridad', 'acceso.minuta.ver'),
    ('supervisor_seguridad', 'acceso.credenciales.gestionar')
) AS v (r, p);
