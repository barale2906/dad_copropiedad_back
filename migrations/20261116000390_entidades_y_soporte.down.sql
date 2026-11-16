DROP FUNCTION entidades_plataforma(timestamptz, uuid, text, uuid, integer);
DROP INDEX ix_entidades_nombre_trgm;
DROP TABLE accesos_soporte;
DELETE FROM roles_permisos
WHERE permiso IN ('plataforma.entidades.usuarios', 'plataforma.soporte.ver');
DELETE FROM permisos WHERE codigo IN ('plataforma.entidades.usuarios', 'plataforma.soporte.ver');
