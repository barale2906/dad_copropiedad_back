DELETE FROM roles_permisos WHERE permiso IN (
    'acceso.correspondencia.entregar', 'acceso.minuta.registrar', 'acceso.minuta.ver',
    'acceso.configurar', 'acceso.credenciales.gestionar', 'acceso.biometria.enrolar');
DELETE FROM permisos WHERE codigo IN (
    'acceso.correspondencia.entregar', 'acceso.minuta.registrar', 'acceso.minuta.ver',
    'acceso.configurar', 'acceso.credenciales.gestionar', 'acceso.biometria.enrolar');

DROP FUNCTION purgar_eventos_acceso(INTEGER);
DROP FUNCTION avisos_acceso_enviados(TIMESTAMPTZ, TIMESTAMPTZ);
DROP FUNCTION outbox_desviar_push(BIGINT, TEXT, TEXT, JSONB);
DROP FUNCTION tokens_push_de_cuentas(UUID[]);
DROP FUNCTION puente_por_credencial(TEXT);
DROP FUNCTION copropiedades_con_control_acceso();
DROP FUNCTION copropiedades_con_acceso_pendiente(TIMESTAMPTZ);

DROP INDEX ix_outbox_concepto;
ALTER TABLE outbox DROP COLUMN concepto;

DROP TABLE dispositivos_push;
DROP TABLE consentimientos_biometricos;
DROP TABLE fotos_eventos_acceso;
DROP TABLE eventos_acceso;
DROP FUNCTION proteger_eventos_acceso();
DROP FUNCTION crear_particiones_eventos_acceso(INTEGER);
DROP FUNCTION ventanas_acceso(UUID, UUID, TIMESTAMPTZ);

DROP TRIGGER trg_reservas_sincronizar ON reservas;
DROP FUNCTION sincronizar_por_reserva();
DROP TRIGGER trg_titularidades_sincronizar ON titularidades;
DROP TRIGGER trg_residencias_sincronizar ON residencias;
DROP FUNCTION sincronizar_por_vinculo();
DROP FUNCTION tocar_credenciales_de_persona(UUID, UUID, UUID);

DROP TABLE autorizaciones_acceso;
DROP FUNCTION sincronizar_por_autorizacion();
DROP TABLE credenciales;
DROP TABLE minuta_adjuntos;
DROP TABLE minuta_porteria;
DROP TABLE purgas_soportes_entrega;
DROP TABLE entregas_correspondencia;
DROP TABLE correspondencia;
DROP TABLE visita_eventos;
DROP TABLE visitas;
DROP FUNCTION credencial_nueva_version();
DROP SEQUENCE acceso_version_seq;
DROP TABLE dispositivos_acceso;
DROP TABLE puentes;
DROP TABLE puntos_acceso;
DROP TABLE configuracion_acceso;
