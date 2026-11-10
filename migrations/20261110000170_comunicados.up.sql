-- Comunicados de la administración (plan 10, fase 4): cartelera digital con destinatarios,
-- lectura y «Enterado». Parte del módulo `gestion_administrativa`.

CREATE TABLE comunicados (
    id                    UUID        PRIMARY KEY,
    tenant_id             UUID        NOT NULL REFERENCES copropiedades (id),
    titulo                TEXT        NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 150),
    cuerpo                TEXT        NOT NULL CHECK (char_length(cuerpo) BETWEEN 1 AND 10000),
    -- Ley 1581: `administracion` llega a todos; `comunicaciones` solo a quien lo autorizó.
    finalidad             TEXT        NOT NULL CHECK (finalidad IN ('administracion', 'comunicaciones')),
    -- `{"tipo": "todos"}`, `{"tipo": "agrupaciones", "ids": [...]}`… (`domain::Audiencia`).
    audiencia             JSONB       NOT NULL CHECK (jsonb_typeof(audiencia) = 'object'),
    importante            BOOLEAN     NOT NULL DEFAULT false,
    requiere_confirmacion BOOLEAN     NOT NULL DEFAULT false,
    estado                TEXT        NOT NULL DEFAULT 'borrador'
                                      CHECK (estado IN ('borrador', 'programado', 'enviado', 'anulado')),
    programado_para       TIMESTAMPTZ,
    enviado_en            TIMESTAMPTZ,
    creado_por            UUID,
    creado_en             TIMESTAMPTZ NOT NULL,
    actualizado_en        TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    CONSTRAINT ck_comunicado_programado CHECK (estado <> 'programado' OR programado_para IS NOT NULL),
    CONSTRAINT ck_comunicado_enviado CHECK ((estado = 'enviado') = (enviado_en IS NOT NULL))
);
CREATE INDEX ix_comunicados_listado ON comunicados (tenant_id, id DESC);
CREATE INDEX ix_comunicados_programados ON comunicados (programado_para) WHERE estado = 'programado';

CREATE TABLE comunicado_adjuntos (
    tenant_id     UUID NOT NULL,
    comunicado_id UUID NOT NULL,
    archivo_id    UUID NOT NULL,
    orden         SMALLINT NOT NULL,
    PRIMARY KEY (tenant_id, comunicado_id, archivo_id),
    FOREIGN KEY (tenant_id, comunicado_id) REFERENCES comunicados (tenant_id, id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, archivo_id)    REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_comunicado_adjuntos_archivo ON comunicado_adjuntos (tenant_id, archivo_id);

-- Una fila por persona al enviarse: los canales por los que salió y la lectura.
CREATE TABLE comunicado_destinatarios (
    tenant_id     UUID        NOT NULL,
    comunicado_id UUID        NOT NULL,
    persona_id    UUID        NOT NULL,
    por_push      BOOLEAN     NOT NULL,
    por_correo    BOOLEAN     NOT NULL,
    por_whatsapp  BOOLEAN     NOT NULL,
    leido_en      TIMESTAMPTZ,
    enterado_en   TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, comunicado_id, persona_id),
    FOREIGN KEY (tenant_id, comunicado_id) REFERENCES comunicados (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)    REFERENCES personas (tenant_id, id),
    CONSTRAINT ck_enterado_tras_leer CHECK (enterado_en IS NULL OR leido_en IS NOT NULL)
);
CREATE INDEX ix_comunicado_destinatarios_persona
    ON comunicado_destinatarios (tenant_id, persona_id, comunicado_id DESC);

-- Personas de la audiencia (adultos o de edad desconocida; con `comunicaciones`, solo las que
-- lo autorizaron). Corre con el tenant de la transacción (RLS): no es SECURITY DEFINER.
CREATE FUNCTION destinatarios_comunicado(p_audiencia JSONB, p_finalidad TEXT, p_ahora TIMESTAMPTZ)
RETURNS TABLE (persona_id UUID, cuenta_id UUID, correo TEXT, telefono TEXT)
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    WITH RECURSIVE
    tipo AS (SELECT p_audiencia ->> 'tipo' AS t),
    ids AS (SELECT (jsonb_array_elements_text(COALESCE(p_audiencia -> 'ids', '[]'::jsonb))) AS v),
    arbol AS (
        SELECT a.id FROM agrupaciones a
        WHERE (SELECT t FROM tipo) = 'agrupaciones' AND a.id::text IN (SELECT v FROM ids)
        UNION
        SELECT h.id FROM agrupaciones h JOIN arbol ON h.padre_id = arbol.id
    ),
    bienes AS (
        SELECT b.id FROM bienes_privados b
        WHERE b.activo AND CASE (SELECT t FROM tipo)
            WHEN 'agrupaciones' THEN b.agrupacion_id IN (SELECT id FROM arbol)
            WHEN 'bienes' THEN b.id::text IN (SELECT v FROM ids)
            ELSE (SELECT t FROM tipo) IN ('todos', 'propietarios', 'residentes')
        END
    ),
    vinculados AS (
        SELECT t.persona_id FROM titularidades t
        WHERE t.bien_privado_id IN (SELECT id FROM bienes)
          AND (SELECT t FROM tipo) <> 'residentes'
          AND t.vigente_desde <= p_ahora AND (t.vigente_hasta IS NULL OR t.vigente_hasta > p_ahora)
        UNION
        SELECT r.persona_id FROM residencias r
        WHERE r.bien_privado_id IN (SELECT id FROM bienes)
          AND (SELECT t FROM tipo) <> 'propietarios'
          AND r.vigente_desde <= p_ahora AND (r.vigente_hasta IS NULL OR r.vigente_hasta > p_ahora)
        UNION
        SELECT p.id FROM personas p
        JOIN asignaciones_rol ar ON ar.cuenta_id = p.cuenta_id
        JOIN roles ro ON ro.id = ar.rol_id
        WHERE (SELECT t FROM tipo) = 'roles' AND ro.codigo IN (SELECT v FROM ids)
          AND ar.vigente_desde <= p_ahora AND (ar.vigente_hasta IS NULL OR ar.vigente_hasta > p_ahora)
    )
    SELECT p.id, p.cuenta_id, p.correo, p.telefono
    FROM personas p
    WHERE p.id IN (SELECT persona_id FROM vinculados)
      AND (p.fecha_nacimiento IS NULL
           OR p.fecha_nacimiento <= (p_ahora AT TIME ZONE 'UTC')::date - INTERVAL '18 years')
      AND (p_finalidad = 'administracion' OR EXISTS (
            SELECT 1 FROM autorizaciones_tratamiento a
            WHERE a.persona_id = p.id AND a.finalidad = p_finalidad AND a.revocada_en IS NULL))
    ORDER BY p.id
$$;
REVOKE ALL ON FUNCTION destinatarios_comunicado(JSONB, TEXT, TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION destinatarios_comunicado(JSONB, TEXT, TIMESTAMPTZ) TO dad_app;

-- El costo de WhatsApp de los comunicados se lleva por PH, como el de control de acceso.
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check
    CHECK (concepto IN ('control_acceso', 'comunicados'));

-- El worker de gestión también envía los comunicados programados que ya vencieron.
CREATE OR REPLACE FUNCTION copropiedades_con_gestion_pendiente()
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT t.tenant_id FROM tickets t
    WHERE (t.estado = 'resuelto' AND t.reabrible_hasta <= now())
       OR (t.vence_en <= now() AND t.alerta_vencimiento_en IS NULL
           AND t.estado NOT IN ('resuelto', 'cerrado', 'rechazado', 'duplicado', 'en_espera_solicitante'))
    UNION
    SELECT p.tenant_id FROM planes_mantenimiento p WHERE p.activo
    UNION
    SELECT c.tenant_id FROM comunicados c WHERE c.estado = 'programado' AND c.programado_para <= now()
$$;

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['comunicados', 'comunicado_adjuntos', 'comunicado_destinatarios'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON comunicados TO dad_app;
GRANT SELECT, INSERT, DELETE ON comunicado_adjuntos TO dad_app;
GRANT SELECT, INSERT ON comunicado_destinatarios TO dad_app;
GRANT UPDATE (leido_en, enterado_en) ON comunicado_destinatarios TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES ('comunicados.gestionar', 'copropiedad');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'comunicados.gestionar', NULL
FROM unnest(ARRAY['administrador', 'asistente_administrativo']) AS r;
