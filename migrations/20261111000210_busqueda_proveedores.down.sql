DELETE FROM roles_permisos WHERE permiso = 'directorio.compartir';
DELETE FROM permisos WHERE codigo = 'directorio.compartir';

DELETE FROM outbox WHERE concepto = 'directorio';
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check
    CHECK (concepto IN ('control_acceso', 'comunicados'));

DROP FUNCTION purgar_solicitudes_proveedor(INTEGER);
DROP FUNCTION numero_whatsapp_publico();

DROP TABLE enlaces_directorio;
DROP TABLE contactos_proveedor;
DROP TABLE solicitudes_proveedor;
