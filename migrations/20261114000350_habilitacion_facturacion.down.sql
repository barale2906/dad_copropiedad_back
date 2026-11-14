DELETE FROM roles_permisos WHERE permiso IN ('facturacion.configurar',
                                             'facturacion.certificado.gestionar');
DELETE FROM permisos WHERE codigo IN ('facturacion.configurar', 'facturacion.certificado.gestionar');

DELETE FROM consecutivos WHERE serie = 'archivo_dian';
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo', 'factura_venta')
           OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

DROP TABLE envios_set_pruebas;
DROP TABLE configuraciones_facturacion;
DROP TABLE certificados_firma;
