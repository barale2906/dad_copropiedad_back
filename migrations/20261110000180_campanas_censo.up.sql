-- Campañas de actualización del censo por WhatsApp (plan 10, fase 5): plantilla
-- `actualizar_datos` con «Están bien» / «Hay cambios»; las respuestas las revisa la
-- administración y aplica los cambios en el censo.

CREATE TABLE campanas_censo (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    creada_por UUID,
    creada_en  TIMESTAMPTZ NOT NULL,
    enviados   INTEGER     NOT NULL CHECK (enviados >= 0),
    UNIQUE (tenant_id, id)
);
CREATE INDEX ix_campanas_censo_tenant ON campanas_censo (tenant_id, id DESC);

-- Una respuesta por persona y bien en cada campaña (la última gana).
CREATE TABLE respuestas_censo (
    tenant_id       UUID        NOT NULL,
    campana_id      UUID        NOT NULL,
    persona_id      UUID        NOT NULL,
    bien_privado_id UUID        NOT NULL,
    respuesta       TEXT        NOT NULL CHECK (respuesta IN ('confirmado', 'cambios')),
    detalle         TEXT        CHECK (char_length(detalle) BETWEEN 1 AND 1000),
    respondida_en   TIMESTAMPTZ NOT NULL,
    atendida_en     TIMESTAMPTZ,
    atendida_por    UUID,
    PRIMARY KEY (tenant_id, campana_id, persona_id, bien_privado_id),
    FOREIGN KEY (tenant_id, campana_id)      REFERENCES campanas_censo (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)      REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id),
    CONSTRAINT ck_respuesta_detalle CHECK ((respuesta = 'cambios') = (detalle IS NOT NULL)),
    CONSTRAINT ck_respuesta_atendida CHECK (atendida_en IS NULL OR respuesta = 'cambios')
);
CREATE INDEX ix_respuestas_censo_pendientes
    ON respuestas_censo (tenant_id, campana_id) WHERE respuesta = 'cambios' AND atendida_en IS NULL;
COMMENT ON COLUMN respuestas_censo.detalle IS
    'dato_personal: cambios del censo descritos por el titular (residentes, vehículos, mascotas)';

-- A quién va la campaña: el responsable de cada bien con residentes y, si nadie reside, el
-- representante de los titulares; adultos (o de edad desconocida) con teléfono. RLS del tenant.
CREATE FUNCTION destinatarios_campana_censo(p_ahora TIMESTAMPTZ)
RETURNS TABLE (persona_id UUID, bien_privado_id UUID, codigo TEXT, telefono TEXT)
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    WITH vinculo AS (
        SELECT r.persona_id, r.bien_privado_id FROM residencias r
        WHERE r.es_responsable
          AND r.vigente_desde <= p_ahora AND (r.vigente_hasta IS NULL OR r.vigente_hasta > p_ahora)
        UNION
        SELECT t.persona_id, t.bien_privado_id FROM titularidades t
        WHERE t.es_representante
          AND t.vigente_desde <= p_ahora AND (t.vigente_hasta IS NULL OR t.vigente_hasta > p_ahora)
          AND NOT EXISTS (
              SELECT 1 FROM residencias r
              WHERE r.bien_privado_id = t.bien_privado_id
                AND r.vigente_desde <= p_ahora
                AND (r.vigente_hasta IS NULL OR r.vigente_hasta > p_ahora))
    )
    SELECT v.persona_id, v.bien_privado_id, b.codigo, p.telefono
    FROM vinculo v
    JOIN personas p ON p.id = v.persona_id
    JOIN bienes_privados b ON b.id = v.bien_privado_id AND b.activo
    WHERE p.telefono IS NOT NULL
      AND (p.fecha_nacimiento IS NULL
           OR p.fecha_nacimiento <= (p_ahora AT TIME ZONE 'UTC')::date - INTERVAL '18 years')
    ORDER BY b.codigo, v.persona_id
$$;
REVOKE ALL ON FUNCTION destinatarios_campana_censo(TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION destinatarios_campana_censo(TIMESTAMPTZ) TO dad_app;

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['campanas_censo', 'respuestas_censo'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT ON campanas_censo TO dad_app;
GRANT SELECT, INSERT, UPDATE ON respuestas_censo TO dad_app;
