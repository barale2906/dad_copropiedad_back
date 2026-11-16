DELETE FROM roles_permisos WHERE permiso IN ('plataforma.comercial.gestionar',
    'plataforma.cobros.aprobar', 'plataforma.cobros.facturar', 'suscripcion.ver');
DELETE FROM permisos WHERE codigo IN ('plataforma.comercial.gestionar',
    'plataforma.cobros.aprobar', 'plataforma.cobros.facturar', 'suscripcion.ver');
DROP FUNCTION medir_consumo(uuid, text, date, date);
DROP TABLE mediciones_consumo;
DROP TABLE cortes_facturacion;
DROP TABLE lineas_suscripcion;
DROP TABLE suscripciones;
DROP TABLE tarifas_producto;
DROP TABLE productos_plataforma;
