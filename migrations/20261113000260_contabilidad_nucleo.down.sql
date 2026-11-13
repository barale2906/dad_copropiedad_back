-- Revierte el núcleo contable.

DELETE FROM roles_permisos WHERE permiso IN (
    'contabilidad.configurar', 'contabilidad.terceros.gestionar',
    'contabilidad.comprobantes.crear', 'contabilidad.comprobantes.contabilizar',
    'contabilidad.comprobantes.revertir', 'contabilidad.periodos.cerrar',
    'contabilidad.periodos.reabrir', 'contabilidad.periodos.aprobar_reapertura',
    'contabilidad.exportar', 'plataforma.plantillas_contables.publicar',
    'plataforma.normativa.gestionar', 'plataforma.entidades.crear');
DELETE FROM permisos WHERE codigo IN (
    'contabilidad.configurar', 'contabilidad.terceros.gestionar',
    'contabilidad.comprobantes.crear', 'contabilidad.comprobantes.contabilizar',
    'contabilidad.comprobantes.revertir', 'contabilidad.periodos.cerrar',
    'contabilidad.periodos.reabrir', 'contabilidad.periodos.aprobar_reapertura',
    'contabilidad.exportar', 'plataforma.plantillas_contables.publicar',
    'plataforma.normativa.gestionar', 'plataforma.entidades.crear');

DROP TABLE notas_estados;
DROP TABLE rubros_estado_financiero;
-- Los triggers de solo inserción impiden TRUNCATE, no DROP.
DROP TABLE lineas_comprobante;
DROP TABLE comprobantes;
DROP FUNCTION exigir_partida_doble();
DROP FUNCTION proteger_lineas_comprobante();
DROP FUNCTION proteger_comprobante();
DROP TABLE lineas_regla;
DROP TABLE reglas_contabilizacion;

DELETE FROM consecutivos WHERE serie LIKE 'comprobante:%';
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo'));

DROP TABLE tipos_comprobante;
DROP TABLE reaperturas_periodo;
DROP TABLE periodos_contables;

DROP INDEX ix_personas_organizacion;
ALTER TABLE personas
    DROP COLUMN organizacion_id,
    DROP COLUMN regimen_iva,
    DROP COLUMN responsabilidades_fiscales,
    DROP COLUMN actividad_ciiu,
    DROP COLUMN direccion_fiscal,
    DROP COLUMN municipio_fiscal_dane,
    DROP COLUMN correo_facturacion;

DROP TABLE configuracion_contable;
DROP TABLE centros_costo;
DROP TABLE cuentas_contables;

DROP FUNCTION plantilla_contable_para_aplicar(UUID, UUID);
DROP FUNCTION plantillas_contables_disponibles(UUID);
DROP TABLE plantillas_contables;
DROP FUNCTION proteger_plantilla_publicada();
DROP TABLE catalogo_puc;

DELETE FROM modulos_habilitados WHERE modulo = 'contabilidad';
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'bienes_comunes', 'control_acceso', 'asambleas', 'gestion_administrativa', 'publicidad'));
