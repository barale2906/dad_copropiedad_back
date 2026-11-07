-- Bienes comunes, parqueaderos y reservas (plan 04, ADR 0008).
--
-- Garantías de la base de datos:
--   1. Dos reservas activas de un recurso exclusivo nunca se solapan (EXCLUDE con btree_gist).
--   2. Solicitar una reserva es idempotente: UNIQUE (tenant_id, idempotency_key).
--   3. Un parqueadero tiene a lo sumo una asignación en cada instante (EXCLUDE sobre vigencias).
--   4. Montos en NUMERIC.
-- El cupo de los recursos con aforo lo controla la aplicación bloqueando el recurso
-- (`FOR UPDATE`) mientras cuenta las personas de la franja.

-- btree_gist es una extensión "trusted": la crea el dueño de la base sin ser superusuario.
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- ---------------------------------------------------------------------
-- Recursos reservables con su política (domain::PoliticaReserva como columnas)
-- ---------------------------------------------------------------------
CREATE TABLE recursos (
    id                               UUID          PRIMARY KEY,
    tenant_id                        UUID          NOT NULL REFERENCES copropiedades (id),
    tipo                             TEXT          NOT NULL CHECK (tipo IN (
                                         'bien_comun', 'parqueadero', 'agenda_administracion',
                                         'herramienta')),
    nombre                           TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    estado                           TEXT          NOT NULL DEFAULT 'disponible' CHECK (estado IN (
                                         'disponible', 'en_mantenimiento', 'fuera_de_servicio',
                                         'inactivo')),
    ubicacion                        TEXT          CHECK (char_length(ubicacion) <= 200),
    descripcion                      TEXT          CHECK (char_length(descripcion) <= 2000),
    modo_asignacion                  TEXT          NOT NULL CHECK (modo_asignacion IN (
                                         'libre', 'franja_inmediata', 'franja_con_aprobacion',
                                         'sorteo', 'turno_rotativo')),
    duracion_minima_min              INTEGER       NOT NULL CHECK (duracion_minima_min > 0),
    duracion_maxima_min              INTEGER       NOT NULL CHECK (duracion_maxima_min >= duracion_minima_min),
    granularidad_min                 INTEGER       NOT NULL CHECK (granularidad_min > 0),
    anticipacion_minima_horas        INTEGER       NOT NULL CHECK (anticipacion_minima_horas >= 0),
    anticipacion_maxima_dias         INTEGER       NOT NULL CHECK (anticipacion_maxima_dias > 0),
    max_reservas_mes                 INTEGER       CHECK (max_reservas_mes > 0),
    max_simultaneas                  INTEGER       CHECK (max_simultaneas > 0),
    alistamiento_min                 INTEGER       NOT NULL CHECK (alistamiento_min >= 0),
    quien_reserva                    TEXT          NOT NULL CHECK (quien_reserva IN ('propietario', 'residente', 'ambos')),
    tenedor_requiere_autorizacion    BOOLEAN       NOT NULL,
    aforo                            INTEGER       CHECK (aforo > 0),
    tarifa                           NUMERIC(14,2) NOT NULL CHECK (tarifa >= 0),
    cobro_por                        TEXT          NOT NULL CHECK (cobro_por IN ('reserva', 'hora')),
    tarifa_festivo                   NUMERIC(14,2) CHECK (tarifa_festivo >= 0),
    deposito                         NUMERIC(14,2) NOT NULL CHECK (deposito >= 0),
    cancelacion_sin_penalidad_horas  INTEGER       NOT NULL CHECK (cancelacion_sin_penalidad_horas >= 0),
    penalidad_porcentaje             NUMERIC(5,2)  NOT NULL CHECK (penalidad_porcentaje BETWEEN 0 AND 100),
    max_invitados                    INTEGER       NOT NULL CHECK (max_invitados >= 0),
    metodo_entrega                   TEXT          NOT NULL CHECK (metodo_entrega IN (
                                         'sin_entrega', 'entrega_llaves', 'entrega_con_inventario',
                                         'acceso_automatico')),
    creado_en                        TIMESTAMPTZ   NOT NULL DEFAULT now(),
    actualizado_en                   TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id)
);
CREATE INDEX ix_recursos_tipo ON recursos (tenant_id, tipo, estado);

-- Horario semanal (domain::Horario), como el de los puntos de atención.
CREATE TABLE franjas_recurso (
    tenant_id        UUID     NOT NULL REFERENCES copropiedades (id),
    recurso_id       UUID     NOT NULL,
    dia_semana       SMALLINT NOT NULL CHECK (dia_semana BETWEEN 1 AND 7),
    hora_inicio      TIME     NOT NULL,
    hora_fin         TIME     NOT NULL CHECK (hora_fin > hora_inicio),
    aplica_festivos  BOOLEAN  NOT NULL DEFAULT FALSE,
    PRIMARY KEY (recurso_id, dia_semana, hora_inicio),
    FOREIGN KEY (tenant_id, recurso_id) REFERENCES recursos (tenant_id, id) ON DELETE CASCADE
);

CREATE TABLE excepciones_recurso (
    tenant_id    UUID NOT NULL REFERENCES copropiedades (id),
    recurso_id   UUID NOT NULL,
    fecha        DATE NOT NULL,
    hora_inicio  TIME,
    hora_fin     TIME,
    motivo       TEXT CHECK (char_length(motivo) <= 200),
    PRIMARY KEY (recurso_id, fecha),
    CHECK ((hora_inicio IS NULL) = (hora_fin IS NULL)),
    CHECK (hora_fin IS NULL OR hora_fin > hora_inicio),
    FOREIGN KEY (tenant_id, recurso_id) REFERENCES recursos (tenant_id, id) ON DELETE CASCADE
);

-- ---------------------------------------------------------------------
-- Bienes comunes (detalle 1:1 del recurso)
-- ---------------------------------------------------------------------
CREATE TABLE bienes_comunes (
    recurso_id        UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    categoria         TEXT          NOT NULL CHECK (categoria IN (
                          'salon_social', 'salon_comunal', 'bbq', 'piscina', 'gimnasio', 'cancha',
                          'sauna', 'turco', 'jacuzzi', 'parque_infantil', 'coworking',
                          'sala_juegos', 'terraza', 'otro')),
    area_m2           NUMERIC(10,2) CHECK (area_m2 > 0),
    -- Art. 59: el uso de un bien común esencial no se restringe por mora.
    es_esencial       BOOLEAN       NOT NULL DEFAULT FALSE,
    reglas            TEXT          CHECK (char_length(reglas) <= 10000),
    reglas_archivo_id UUID,
    FOREIGN KEY (tenant_id, recurso_id)        REFERENCES recursos (tenant_id, id),
    FOREIGN KEY (tenant_id, reglas_archivo_id) REFERENCES archivos (tenant_id, id)
);

-- Un uso exclusivo (art. 22) puede señalar el bien común del catálogo (pendiente del plan 03).
ALTER TABLE usos_exclusivos
    ADD COLUMN recurso_id UUID,
    ADD CONSTRAINT fk_usos_exclusivos_recurso
        FOREIGN KEY (tenant_id, recurso_id) REFERENCES recursos (tenant_id, id);

-- ---------------------------------------------------------------------
-- Parqueaderos y sus asignaciones
-- ---------------------------------------------------------------------
CREATE TABLE parqueaderos (
    recurso_id       UUID PRIMARY KEY,
    tenant_id        UUID NOT NULL REFERENCES copropiedades (id),
    codigo           TEXT NOT NULL CHECK (char_length(codigo) BETWEEN 1 AND 20),
    tipo_vehiculo    TEXT NOT NULL CHECK (tipo_vehiculo IN (
                         'carro', 'moto', 'bicicleta', 'discapacidad', 'electrico')),
    naturaleza       TEXT NOT NULL CHECK (naturaleza IN (
                         'privado', 'uso_exclusivo', 'comunal', 'visitantes')),
    -- Bien privado que es (o del que es anexo) el parqueadero privado.
    bien_privado_id  UUID,
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, recurso_id),
    CHECK ((naturaleza = 'privado') = (bien_privado_id IS NOT NULL)),
    FOREIGN KEY (tenant_id, recurso_id)      REFERENCES recursos (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);

CREATE TABLE sorteos (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    nombre         TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    -- Parqueaderos (u otros recursos) que se adjudican.
    recursos       UUID[]      NOT NULL CHECK (cardinality(recursos) > 0),
    -- Periodo de las asignaciones que resultan (rotación de 3, 6 o 12 meses…).
    vigencia       TSTZRANGE   NOT NULL CHECK (NOT isempty(vigencia) AND upper(vigencia) IS NOT NULL),
    estado         TEXT        NOT NULL DEFAULT 'abierto' CHECK (estado IN ('abierto', 'ejecutado')),
    -- SHA-256 de la semilla, publicado al crear el sorteo.
    compromiso     TEXT        NOT NULL CHECK (compromiso ~ '^[0-9a-f]{64}$'),
    -- Secreta hasta la ejecución: la API no la muestra antes.
    semilla        TEXT        NOT NULL CHECK (semilla ~ '^[0-9a-f]{64}$'),
    criterios      JSONB       NOT NULL,
    -- Orden de los bienes privados inscritos tras ejecutar (el primero, el primer favorecido).
    resultado      UUID[],
    creado_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    ejecutado_en   TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'ejecutado') = (ejecutado_en IS NOT NULL AND resultado IS NOT NULL))
);
COMMENT ON COLUMN sorteos.semilla IS 'secreto hasta ejecutar el sorteo (compromiso-revelación)';

CREATE TABLE inscripciones_sorteo (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    sorteo_id        UUID        NOT NULL,
    bien_privado_id  UUID        NOT NULL,
    discapacidad     BOOLEAN     NOT NULL DEFAULT FALSE,
    inscrito_por     UUID,
    inscrito_en      TIMESTAMPTZ NOT NULL,
    UNIQUE (sorteo_id, bien_privado_id),
    FOREIGN KEY (tenant_id, sorteo_id)       REFERENCES sorteos (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);
COMMENT ON COLUMN inscripciones_sorteo.discapacidad IS 'dato_sensible: condición de salud de un residente';

CREATE TABLE asignaciones_parqueadero (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    recurso_id       UUID        NOT NULL,
    bien_privado_id  UUID        NOT NULL,
    vehiculo_id      UUID,
    origen           TEXT        NOT NULL CHECK (origen IN ('propiedad', 'sorteo', 'arriendo', 'manual')),
    vigencia         TSTZRANGE   NOT NULL CHECK (NOT isempty(vigencia)),
    sorteo_id        UUID,
    creado_en        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    CHECK ((origen = 'sorteo') = (sorteo_id IS NOT NULL)),
    EXCLUDE USING gist (recurso_id WITH =, vigencia WITH &&),
    FOREIGN KEY (tenant_id, recurso_id)      REFERENCES parqueaderos (tenant_id, recurso_id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, vehiculo_id)     REFERENCES vehiculos (tenant_id, id),
    FOREIGN KEY (tenant_id, sorteo_id)       REFERENCES sorteos (tenant_id, id)
);
CREATE INDEX ix_asignaciones_parqueadero_bien ON asignaciones_parqueadero (tenant_id, bien_privado_id);

-- ---------------------------------------------------------------------
-- Restricciones de reserva (mora o sanción, marca manual hasta el módulo de contabilidad)
-- ---------------------------------------------------------------------
CREATE TABLE restricciones_reserva (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id     UUID        NOT NULL,
    motivo              TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 1 AND 500),
    soporte_archivo_id  UUID,
    desde               TIMESTAMPTZ NOT NULL,
    hasta               TIMESTAMPTZ CHECK (hasta > desde),
    creada_por          UUID,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)    REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_restricciones_bien ON restricciones_reserva (tenant_id, bien_privado_id, desde);

-- El propietario autoriza (o no) que su tenedor reserve, cuando la política lo exige.
ALTER TABLE residencias ADD COLUMN reservas_autorizadas BOOLEAN NOT NULL DEFAULT FALSE;

-- ---------------------------------------------------------------------
-- Reservas (y citas con la administración)
-- ---------------------------------------------------------------------
CREATE TABLE reservas (
    id                       UUID          PRIMARY KEY,
    tenant_id                UUID          NOT NULL REFERENCES copropiedades (id),
    recurso_id               UUID          NOT NULL,
    bien_privado_id          UUID          NOT NULL,
    solicitante_persona_id   UUID,
    solicitada_por           UUID,         -- cuenta
    franja                   TSTZRANGE     NOT NULL CHECK (NOT isempty(franja)),
    -- Copia de la política al reservar: exclusiva (sin aforo) o por cupo.
    exclusiva                BOOLEAN       NOT NULL,
    invitados                INTEGER       NOT NULL DEFAULT 0 CHECK (invitados >= 0),
    estado                   TEXT          NOT NULL CHECK (estado IN (
                                 'solicitada', 'aprobada', 'rechazada', 'en_curso', 'finalizada',
                                 'con_novedad', 'cancelada', 'no_asistio')),
    valor                    NUMERIC(14,2) NOT NULL CHECK (valor >= 0),
    deposito                 NUMERIC(14,2) NOT NULL CHECK (deposito >= 0),
    penalidad                NUMERIC(14,2) NOT NULL DEFAULT 0 CHECK (penalidad >= 0),
    estado_pago              TEXT          NOT NULL CHECK (estado_pago IN (
                                 'no_aplica', 'pendiente', 'pagado', 'exonerado')),
    idempotency_key          TEXT          NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    canal                    TEXT          NOT NULL CHECK (canal IN ('web', 'whatsapp', 'presencial')),
    -- Citas con la administración.
    modalidad                TEXT          CHECK (modalidad IN ('presencial', 'llamada', 'videollamada')),
    motivo                   TEXT          CHECK (char_length(motivo) <= 500),
    -- Parqueaderos de visitantes.
    placa_visitante          TEXT          CHECK (placa_visitante ~ '^[A-Z0-9]{4,8}$'),
    -- Motivo de rechazo o cancelación.
    observacion              TEXT          CHECK (char_length(observacion) <= 500),
    recordatorio_enviado_en  TIMESTAMPTZ,
    creada_en                TIMESTAMPTZ   NOT NULL,
    actualizada_en           TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key),
    CONSTRAINT ex_reservas_sin_solape EXCLUDE USING gist (recurso_id WITH =, franja WITH &&)
        WHERE (exclusiva AND estado IN ('solicitada', 'aprobada', 'en_curso')),
    FOREIGN KEY (tenant_id, recurso_id)             REFERENCES recursos (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)        REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, solicitante_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_reservas_recurso ON reservas USING gist (tenant_id, recurso_id, franja);
CREATE INDEX ix_reservas_bien ON reservas (tenant_id, bien_privado_id, lower(franja));
CREATE INDEX ix_reservas_recordatorio ON reservas (lower(franja))
    WHERE estado = 'aprobada' AND recordatorio_enviado_en IS NULL;
COMMENT ON COLUMN reservas.placa_visitante IS 'dato_personal: identifica al visitante';
COMMENT ON COLUMN reservas.motivo IS 'dato_personal: texto libre del solicitante';

-- Entrega y recibo con checklist, fotos y novedades.
CREATE TABLE entregas_reserva (
    id              UUID          PRIMARY KEY,
    tenant_id       UUID          NOT NULL REFERENCES copropiedades (id),
    reserva_id      UUID          NOT NULL,
    tipo            TEXT          NOT NULL CHECK (tipo IN ('entrega', 'recibo')),
    -- [{ "elemento": "Sillas", "cantidad": 40, "estado": "bueno" }]
    checklist       JSONB         NOT NULL DEFAULT '[]'::jsonb,
    fotos           UUID[]        NOT NULL DEFAULT '{}',
    novedad         TEXT          CHECK (char_length(novedad) <= 2000),
    valor_danos     NUMERIC(14,2) CHECK (valor_danos > 0),
    registrado_por  UUID,
    registrado_en   TIMESTAMPTZ   NOT NULL,
    UNIQUE (reserva_id, tipo),
    FOREIGN KEY (tenant_id, reserva_id) REFERENCES reservas (tenant_id, id)
);

-- Cargos pendientes para el futuro módulo de contabilidad (daños, penalidades).
CREATE TABLE cargos_pendientes (
    id               UUID          PRIMARY KEY,
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    bien_privado_id  UUID          NOT NULL,
    concepto         TEXT          NOT NULL CHECK (concepto IN ('danos', 'penalidad_cancelacion')),
    descripcion      TEXT          CHECK (char_length(descripcion) <= 500),
    valor            NUMERIC(14,2) NOT NULL CHECK (valor > 0),
    reserva_id       UUID,
    estado           TEXT          NOT NULL DEFAULT 'pendiente'
                     CHECK (estado IN ('pendiente', 'aplicado', 'anulado')),
    creado_en        TIMESTAMPTZ   NOT NULL,
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, reserva_id)      REFERENCES reservas (tenant_id, id)
);
CREATE INDEX ix_cargos_pendientes_bien ON cargos_pendientes (tenant_id, bien_privado_id, estado);

-- ---------------------------------------------------------------------
-- Recordatorios: copropiedades con reservas aprobadas que empiezan pronto y aún no se
-- recordaron (lo consulta el worker del gateway; solo devuelve ids de tenant).
-- ---------------------------------------------------------------------
CREATE FUNCTION copropiedades_con_recordatorios(p_hasta TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT r.tenant_id FROM reservas r
    WHERE r.estado = 'aprobada' AND r.recordatorio_enviado_en IS NULL
      AND lower(r.franja) > now() AND lower(r.franja) <= p_hasta
$$;
REVOKE ALL ON FUNCTION copropiedades_con_recordatorios(TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION copropiedades_con_recordatorios(TIMESTAMPTZ) TO dad_app;

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'recursos', 'franjas_recurso', 'excepciones_recurso', 'bienes_comunes', 'parqueaderos',
        'sorteos', 'inscripciones_sorteo', 'asignaciones_parqueadero', 'restricciones_reserva',
        'reservas', 'entregas_reserva', 'cargos_pendientes'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT, UPDATE ON recursos, bienes_comunes, parqueaderos TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON franjas_recurso, excepciones_recurso TO dad_app;
GRANT SELECT, INSERT ON sorteos, inscripciones_sorteo, asignaciones_parqueadero,
    restricciones_reserva, reservas, entregas_reserva, cargos_pendientes TO dad_app;
GRANT UPDATE (estado, resultado, ejecutado_en) ON sorteos TO dad_app;
GRANT UPDATE (vigencia) ON asignaciones_parqueadero TO dad_app;
GRANT UPDATE (hasta) ON restricciones_reserva TO dad_app;
GRANT UPDATE (estado, penalidad, estado_pago, observacion, recordatorio_enviado_en,
              actualizada_en) ON reservas TO dad_app;
GRANT UPDATE (reservas_autorizadas) ON residencias TO dad_app;
GRANT UPDATE (recurso_id) ON usos_exclusivos TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('reservas.ver_todas', 'copropiedad'),
    ('reservas.cancelar_cualquiera', 'copropiedad'),
    ('reservas.entregas.registrar', 'copropiedad'),
    ('reservas.restringir', 'copropiedad'),
    ('parqueaderos.ver', 'copropiedad'),
    ('parqueaderos.asignar', 'copropiedad'),
    ('sorteos.gestionar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('administrador'), 'copropiedad', p, NULL
FROM unnest(ARRAY['reservas.ver_todas', 'reservas.cancelar_cualquiera',
                  'reservas.entregas.registrar', 'reservas.restringir', 'parqueaderos.ver',
                  'parqueaderos.asignar', 'sorteos.gestionar']) AS p;
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'reservas.ver_todas', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'reservas.cancelar_cualquiera', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'reservas.entregas.registrar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'parqueaderos.ver', NULL),
    (uuid_rol_plantilla('vigilante'), 'copropiedad', 'reservas.entregas.registrar', NULL),
    (uuid_rol_plantilla('vigilante'), 'copropiedad', 'parqueaderos.ver', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'reservas.entregas.registrar', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'parqueaderos.ver', NULL);
