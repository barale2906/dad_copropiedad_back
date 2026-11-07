DELETE FROM roles_permisos WHERE permiso IN ('censo.ver', 'censo.editar', 'censo.aprobar', 'censo.exportar');
DELETE FROM permisos WHERE codigo IN ('censo.ver', 'censo.editar', 'censo.aprobar', 'censo.exportar');

DROP TABLE cambios_censo;
DROP FUNCTION exigir_cambio_pendiente();
DROP TABLE mascotas;
DROP TABLE vehiculos;
DROP TABLE residencias;

DROP INDEX ix_personas_cuenta;
ALTER TABLE personas
    DROP COLUMN actualizado_en,
    DROP COLUMN movilidad_reducida,
    DROP COLUMN contacto_emergencia_telefono,
    DROP COLUMN contacto_emergencia_nombre,
    DROP COLUMN fecha_nacimiento,
    DROP COLUMN tipo_documento;
