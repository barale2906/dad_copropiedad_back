DROP FUNCTION organizaciones_de_cuenta(UUID);
DROP FUNCTION crear_organizacion(UUID, TEXT, TEXT, TEXT, TEXT[], TEXT, TEXT, UUID);
DROP FUNCTION buscar_organizacion_por_nit(TEXT);
DROP FUNCTION buscar_cuenta(TEXT);
DROP FUNCTION membresias_de_cuenta(UUID);
REVOKE INSERT ON copropiedades FROM dad_app;
DROP POLICY copropiedades_plataforma ON copropiedades;

DROP TABLE contratos_servicio;
DROP TABLE reclamaciones_organizacion;
DROP TABLE organizaciones_miembros;
DROP TABLE organizaciones;
DROP TABLE asignaciones_plataforma;
DROP TABLE asignaciones_rol;
DROP FUNCTION validar_asignacion_rol();
DROP TABLE roles_permisos;
DROP TABLE roles;
DROP FUNCTION uuid_rol_plantilla(TEXT);
DROP TABLE permisos;

DROP POLICY outbox_plataforma ON outbox;
DELETE FROM outbox WHERE tenant_id IS NULL;
ALTER TABLE outbox ALTER COLUMN tenant_id SET NOT NULL;

DROP TABLE auditoria_plataforma;
DROP TABLE sesiones;
DROP TABLE codigos_verificacion;
DROP TABLE cuentas;
DROP FUNCTION app_plataforma();
