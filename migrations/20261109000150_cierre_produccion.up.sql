-- Cierre para producción (plan 09):
--   1. Solicitudes del titular con ciclo completo (Ley 1581, arts. 14 y 15): detalle, quién
--      radicó, prórroga única con motivo, respuesta con texto o archivo.
--   2. Índices para consultar la auditoría por entidad, actor y acción con cursor por id.

ALTER TABLE solicitudes_titular
    ADD COLUMN detalle              TEXT        CHECK (char_length(detalle) <= 2000),
    ADD COLUMN radicada_por         UUID        REFERENCES cuentas (id),
    ADD COLUMN prorrogada_hasta     DATE,
    ADD COLUMN motivo_prorroga      TEXT        CHECK (char_length(motivo_prorroga) <= 500),
    ADD COLUMN respuesta            TEXT        CHECK (char_length(respuesta) <= 5000),
    ADD COLUMN respuesta_archivo_id UUID,
    ADD COLUMN respondida_en        TIMESTAMPTZ,
    ADD COLUMN respondida_por       UUID        REFERENCES cuentas (id),
    ADD UNIQUE (tenant_id, id),
    ADD CONSTRAINT fk_solicitud_respuesta_archivo
        FOREIGN KEY (tenant_id, respuesta_archivo_id) REFERENCES archivos (tenant_id, id),
    ADD CONSTRAINT ck_solicitud_respondida
        CHECK ((estado = 'respondida') = (respondida_en IS NOT NULL)),
    ADD CONSTRAINT ck_solicitud_respuesta
        CHECK (estado <> 'respondida' OR respuesta IS NOT NULL OR respuesta_archivo_id IS NOT NULL),
    ADD CONSTRAINT ck_solicitud_prorroga
        CHECK ((prorrogada_hasta IS NULL) = (motivo_prorroga IS NULL) AND
               (prorrogada_hasta IS NULL OR prorrogada_hasta > vence_el));

COMMENT ON COLUMN solicitudes_titular.detalle IS 'dato_personal: lo que pide el titular sobre sus datos';
COMMENT ON COLUMN solicitudes_titular.respuesta IS 'dato_personal: respuesta al titular';

CREATE INDEX ix_solicitudes_titular_persona ON solicitudes_titular (tenant_id, persona_id, radicada_en DESC);

GRANT UPDATE (estado, prorrogada_hasta, motivo_prorroga, respuesta, respuesta_archivo_id,
              respondida_en, respondida_por) ON solicitudes_titular TO dad_app;

CREATE INDEX ix_auditoria_cursor ON auditoria (tenant_id, id DESC);
CREATE INDEX ix_auditoria_actor  ON auditoria (tenant_id, actor_id, id DESC) WHERE actor_id IS NOT NULL;
CREATE INDEX ix_auditoria_accion ON auditoria (tenant_id, accion, id DESC);
