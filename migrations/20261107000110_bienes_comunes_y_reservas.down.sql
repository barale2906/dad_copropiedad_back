DELETE FROM roles_permisos WHERE permiso IN (
    'reservas.ver_todas', 'reservas.cancelar_cualquiera', 'reservas.entregas.registrar',
    'reservas.restringir', 'parqueaderos.ver', 'parqueaderos.asignar', 'sorteos.gestionar');
DELETE FROM permisos WHERE codigo IN (
    'reservas.ver_todas', 'reservas.cancelar_cualquiera', 'reservas.entregas.registrar',
    'reservas.restringir', 'parqueaderos.ver', 'parqueaderos.asignar', 'sorteos.gestionar');

DROP FUNCTION copropiedades_con_recordatorios(TIMESTAMPTZ);
DROP TABLE cargos_pendientes;
DROP TABLE entregas_reserva;
DROP TABLE reservas;
ALTER TABLE residencias DROP COLUMN reservas_autorizadas;
DROP TABLE restricciones_reserva;
DROP TABLE asignaciones_parqueadero;
DROP TABLE inscripciones_sorteo;
DROP TABLE sorteos;
DROP TABLE parqueaderos;
ALTER TABLE usos_exclusivos DROP CONSTRAINT fk_usos_exclusivos_recurso, DROP COLUMN recurso_id;
DROP TABLE bienes_comunes;
DROP TABLE excepciones_recurso;
DROP TABLE franjas_recurso;
DROP TABLE recursos;
-- btree_gist se conserva: otras migraciones futuras pueden usarla y quitarla no aporta nada.
