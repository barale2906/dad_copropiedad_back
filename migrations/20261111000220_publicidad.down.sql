DELETE FROM roles_permisos WHERE permiso = 'plataforma.publicidad.liquidar';
DELETE FROM permisos WHERE codigo = 'plataforma.publicidad.liquidar';

DROP FUNCTION atender_reportes_publicidad(UUID, TIMESTAMPTZ);
DROP FUNCTION reportes_publicidad_pendientes(UUID, INTEGER);
DROP FUNCTION consumo_directo_anunciantes(TIMESTAMPTZ, TIMESTAMPTZ);
DROP FUNCTION leads_anunciante(UUID, UUID, INTEGER);
DROP FUNCTION alcance_campana(UUID);
DROP FUNCTION conflicto_interes_campana(UUID);
DROP FUNCTION copropiedades_con_publicidad(TIMESTAMPTZ);
DROP FUNCTION aplicar_entrega_publicidad(UUID, TEXT, TIMESTAMPTZ, INTEGER, INTEGER, INTEGER, NUMERIC, BOOLEAN);
DROP FUNCTION pieza_para_entrega(UUID, TIMESTAMPTZ);
DROP FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ);
DROP FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ);
DROP FUNCTION dia_publicidad(TIMESTAMPTZ);
DROP FUNCTION campana_alcanza(UUID, UUID);

DELETE FROM outbox WHERE concepto = 'publicidad';
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check
    CHECK (concepto IN ('control_acceso', 'comunicados', 'directorio'));
ALTER TABLE outbox DROP COLUMN campana_id;

DROP TABLE pantallas_publicidad;
DROP TABLE reportes_publicidad;
DROP TABLE envios_publicidad;
DROP TABLE liquidaciones;
DROP TABLE entregas_publicidad;
DROP TABLE tarifas_publicidad;
DROP TABLE creativos;
DROP TABLE campana_copropiedades;
DROP TABLE campanas;

DELETE FROM autorizaciones_tratamiento WHERE finalidad = 'publicidad' AND politica_id IS NULL;
ALTER TABLE autorizaciones_tratamiento DROP CONSTRAINT ck_autorizacion_politica;
ALTER TABLE autorizaciones_tratamiento DROP COLUMN politica_publicidad_id;
ALTER TABLE autorizaciones_tratamiento ALTER COLUMN politica_id SET NOT NULL;
DROP TABLE politicas_publicidad;
