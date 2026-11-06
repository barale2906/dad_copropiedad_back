-- Revierte el esquema inicial. El rol `dad_app` es global al clúster y NO se elimina aquí
-- (otras bases, como las de `#[sqlx::test]`, pueden usarlo).
DROP FUNCTION IF EXISTS outbox_marcar_fallido(BIGINT, TEXT, INTERVAL, INTEGER);
DROP FUNCTION IF EXISTS outbox_marcar_enviado(BIGINT, TEXT);
DROP FUNCTION IF EXISTS outbox_reclamar(INTEGER);
DROP FUNCTION IF EXISTS reuniones_activas_por_telefono(TEXT, TEXT);

DROP TABLE IF EXISTS auditoria;
DROP TABLE IF EXISTS outbox;
DROP TABLE IF EXISTS votos;
DROP TABLE IF EXISTS participaciones;
DROP TABLE IF EXISTS opciones;
DROP TABLE IF EXISTS preguntas;
DROP TABLE IF EXISTS inscripciones;
DROP TABLE IF EXISTS reuniones;
DROP TABLE IF EXISTS personas;
DROP TABLE IF EXISTS inmuebles;
DROP TABLE IF EXISTS copropiedades;
DROP TABLE IF EXISTS lineas_whatsapp;

DROP FUNCTION IF EXISTS validar_voto();
DROP FUNCTION IF EXISTS validar_participacion();
DROP FUNCTION IF EXISTS rechazar_modificacion();
DROP FUNCTION IF EXISTS app_tenant_id();

REVOKE USAGE ON SCHEMA public FROM dad_app;
