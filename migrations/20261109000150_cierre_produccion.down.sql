DROP INDEX ix_auditoria_accion;
DROP INDEX ix_auditoria_actor;
DROP INDEX ix_auditoria_cursor;

REVOKE UPDATE (estado, prorrogada_hasta, motivo_prorroga, respuesta, respuesta_archivo_id,
               respondida_en, respondida_por) ON solicitudes_titular FROM dad_app;
DROP INDEX ix_solicitudes_titular_persona;
ALTER TABLE solicitudes_titular
    DROP CONSTRAINT ck_solicitud_prorroga,
    DROP CONSTRAINT ck_solicitud_respuesta,
    DROP CONSTRAINT ck_solicitud_respondida,
    DROP CONSTRAINT fk_solicitud_respuesta_archivo,
    DROP CONSTRAINT solicitudes_titular_tenant_id_id_key,
    DROP COLUMN respondida_por,
    DROP COLUMN respondida_en,
    DROP COLUMN respuesta_archivo_id,
    DROP COLUMN respuesta,
    DROP COLUMN motivo_prorroga,
    DROP COLUMN prorrogada_hasta,
    DROP COLUMN radicada_por,
    DROP COLUMN detalle;
