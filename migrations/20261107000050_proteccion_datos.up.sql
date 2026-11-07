-- Protección de datos personales (Ley 1581 de 2012, Decreto 1377 de 2013). Plan 01 §5.
-- Cada copropiedad es responsable del tratamiento; la plataforma, encargada.

-- Versiones de la política de tratamiento. Inmutables: un cambio es una versión nueva.
CREATE TABLE politicas_tratamiento (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    version        INTEGER     NOT NULL CHECK (version > 0),
    texto          TEXT,
    archivo_id     UUID,
    vigente_desde  TIMESTAMPTZ NOT NULL,
    creado_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, version),
    UNIQUE (tenant_id, id),
    CHECK (texto IS NOT NULL OR archivo_id IS NOT NULL),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);

-- Una fila por persona y finalidad. Revocar = fijar `revocada_en` (se conserva la prueba de
-- que existió la autorización). Solo una vigente por persona y finalidad.
CREATE TABLE autorizaciones_tratamiento (
    id                    UUID        PRIMARY KEY,
    tenant_id             UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id            UUID        NOT NULL,
    politica_id           UUID        NOT NULL,
    finalidad             TEXT        NOT NULL CHECK (finalidad IN (
                              'administracion', 'comunicaciones', 'control_acceso', 'publicidad')),
    medio                 TEXT        NOT NULL CHECK (medio IN ('web', 'app', 'whatsapp', 'escrito')),
    evidencia_archivo_id  UUID,
    otorgada_en           TIMESTAMPTZ NOT NULL,
    revocada_en           TIMESTAMPTZ CHECK (revocada_en >= otorgada_en),
    FOREIGN KEY (tenant_id, persona_id)           REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, politica_id)          REFERENCES politicas_tratamiento (tenant_id, id),
    FOREIGN KEY (tenant_id, evidencia_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_autorizaciones_vigentes
    ON autorizaciones_tratamiento (tenant_id, persona_id, finalidad) WHERE revocada_en IS NULL;

-- Consultas (art. 14) y reclamos (art. 15) de los titulares. El detalle de la petición y la
-- respuesta llegarán con el módulo de tickets (plan 07).
CREATE TABLE solicitudes_titular (
    id           UUID        PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id   UUID        NOT NULL,
    tipo         TEXT        NOT NULL CHECK (tipo IN (
                     'consulta', 'actualizacion', 'rectificacion', 'supresion', 'revocatoria')),
    radicada_en  TIMESTAMPTZ NOT NULL,
    vence_el     DATE        NOT NULL,
    estado       TEXT        NOT NULL DEFAULT 'radicada'
                 CHECK (estado IN ('radicada', 'en_tramite', 'respondida')),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
-- Seguimiento de vencimientos por copropiedad.
CREATE INDEX ix_solicitudes_titular_vence ON solicitudes_titular (tenant_id, estado, vence_el);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'politicas_tratamiento', 'autorizaciones_tratamiento', 'solicitudes_titular'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT ON politicas_tratamiento, autorizaciones_tratamiento, solicitudes_titular
    TO dad_app;
GRANT UPDATE (revocada_en) ON autorizaciones_tratamiento TO dad_app;

-- Clasificación de columnas con datos personales (prefijo `dato_personal:` o
-- `dato_sensible:`). La prueba `columnas_personales_estan_clasificadas` exige el comentario.
COMMENT ON COLUMN personas.nombre IS 'dato_personal: nombre del titular';
COMMENT ON COLUMN personas.documento IS 'dato_personal: documento de identidad';
COMMENT ON COLUMN personas.telefono IS 'dato_personal: celular (E.164 sin +)';
COMMENT ON COLUMN personas.correo IS 'dato_personal: correo electrónico';
COMMENT ON COLUMN archivos.nombre_original IS 'dato_personal: puede contener nombres o documentos';
COMMENT ON COLUMN outbox.destino IS 'dato_personal: teléfono o correo del destinatario';
COMMENT ON COLUMN outbox.payload IS 'dato_personal: contenido del mensaje';
COMMENT ON COLUMN outbox.ultimo_error IS 'dato_personal: el error del proveedor puede incluir el destino';
