COMMENT ON COLUMN personas.nombre IS NULL;
COMMENT ON COLUMN personas.documento IS NULL;
COMMENT ON COLUMN personas.telefono IS NULL;
COMMENT ON COLUMN personas.correo IS NULL;
COMMENT ON COLUMN archivos.nombre_original IS NULL;
COMMENT ON COLUMN outbox.destino IS NULL;
COMMENT ON COLUMN outbox.payload IS NULL;
COMMENT ON COLUMN outbox.ultimo_error IS NULL;

DROP TABLE solicitudes_titular;
DROP TABLE autorizaciones_tratamiento;
DROP TABLE politicas_tratamiento;
