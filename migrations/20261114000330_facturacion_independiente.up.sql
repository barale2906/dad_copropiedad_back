-- Facturación electrónica como módulo independiente (plan 18 fase A2, ADR 0022).
--
-- Una entidad puede facturar sin el módulo de contabilidad: la cuenta contable de cada línea y
-- el comprobante de la factura pasan a ser opcionales (NULL = sin contabilizar), la factura de
-- venta toma su número interno de `consecutivos` (el legal es el de la DIAN, en
-- `documentos_electronicos`), aparece el catálogo de productos y servicios y la marca de pago
-- manual para quien no lleva tesorería. Con contabilidad nada cambia: el documento se
-- contabiliza como antes.

-- ---------------------------------------------------------------------
-- Catálogo de productos y servicios
-- ---------------------------------------------------------------------
CREATE TABLE items_facturacion (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES entidades (id),
    codigo            TEXT          NOT NULL CHECK (codigo ~ '^[A-Za-z0-9._-]{1,30}$'),
    descripcion       TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    -- Código de unidad de medida del catálogo de la DIAN (UN/ECE rec. 20; 94 = unidad).
    unidad            TEXT          NOT NULL DEFAULT '94' CHECK (unidad ~ '^[A-Z0-9]{1,3}$'),
    -- Precio sugerido; cada línea lleva el suyo.
    precio            NUMERIC(18,2) CHECK (precio > 0),
    tarifa_iva        NUMERIC(5,2)  NOT NULL DEFAULT 0 CHECK (tarifa_iva >= 0 AND tarifa_iva <= 100),
    clasificacion     TEXT          NOT NULL DEFAULT 'venta' CHECK (clasificacion IN (
                          'expensa', 'cuota_extraordinaria', 'uso_sin_cobro', 'alquiler_servicio',
                          'venta', 'otra')),
    -- Cuenta de ingreso: solo la usa quien tiene contabilidad.
    cuenta_id         UUID,
    activo            BOOLEAN       NOT NULL DEFAULT TRUE,
    creado_por        UUID,
    creado_en         TIMESTAMPTZ   NOT NULL,
    actualizado_en    TIMESTAMPTZ   NOT NULL,
    CHECK (tarifa_iva = 0 OR clasificacion IN ('alquiler_servicio', 'venta')),
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_id) REFERENCES cuentas_contables (tenant_id, id)
);
CREATE INDEX ix_items_facturacion_descripcion
    ON items_facturacion USING gin (normalizar(descripcion) gin_trgm_ops);
CREATE INDEX ix_items_facturacion_cuenta ON items_facturacion (tenant_id, cuenta_id)
    WHERE cuenta_id IS NOT NULL;

ALTER TABLE items_facturacion ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON items_facturacion
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
GRANT SELECT, INSERT, UPDATE ON items_facturacion TO dad_app;

-- ---------------------------------------------------------------------
-- Facturas de venta sin contabilidad
-- ---------------------------------------------------------------------
ALTER TABLE lineas_factura_venta
    ALTER COLUMN cuenta_id DROP NOT NULL,
    ADD COLUMN item_id UUID,
    ADD CONSTRAINT fk_linea_venta_item
        FOREIGN KEY (tenant_id, item_id) REFERENCES items_facturacion (tenant_id, id);
CREATE INDEX ix_lineas_venta_item ON lineas_factura_venta (tenant_id, item_id)
    WHERE item_id IS NOT NULL;

-- Emitida = número interno y fecha de emisión; el comprobante solo si se contabilizó.
ALTER TABLE facturas_venta DROP CONSTRAINT facturas_venta_check5;
ALTER TABLE facturas_venta ADD CONSTRAINT ck_factura_venta_emitida
    CHECK (estado IN ('borrador', 'anulada') OR (numero IS NOT NULL AND emitida_en IS NOT NULL));

-- Marca de pago manual: solo para quien factura sin tesorería (lo exige el caso de uso).
ALTER TABLE facturas_venta
    ADD COLUMN pago_manual_fecha       DATE,
    ADD COLUMN pago_manual_medio       TEXT CHECK (pago_manual_medio IN (
                                           'efectivo', 'transferencia', 'consignacion', 'tarjeta',
                                           'cheque', 'pse', 'otro')),
    ADD COLUMN pago_manual_referencia  TEXT CHECK (char_length(pago_manual_referencia) BETWEEN 1 AND 100),
    ADD COLUMN pago_manual_por         UUID,
    ADD CONSTRAINT ck_pago_manual_completo
        CHECK ((pago_manual_fecha IS NULL) = (pago_manual_medio IS NULL)
               AND (pago_manual_fecha IS NULL) = (pago_manual_por IS NULL)
               AND (pago_manual_referencia IS NULL OR pago_manual_fecha IS NOT NULL)),
    ADD CONSTRAINT ck_pago_manual_pagada
        CHECK (pago_manual_fecha IS NULL OR (estado IN ('pagada', 'anulada') AND comprobante_id IS NULL));
COMMENT ON COLUMN facturas_venta.pago_manual_referencia IS
    'dato_personal: referencia del pago del cliente (puede identificar su cuenta)';

-- Pendientes de contabilizar (al habilitar la contabilidad después).
CREATE INDEX ix_facturas_venta_sin_contabilizar ON facturas_venta (tenant_id, fecha)
    WHERE comprobante_id IS NULL AND estado NOT IN ('borrador', 'anulada');

-- Número interno por año.
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo', 'factura_venta')
           OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

-- ---------------------------------------------------------------------
-- Compras (documento soporte) sin contabilidad
-- ---------------------------------------------------------------------
ALTER TABLE lineas_factura_proveedor ALTER COLUMN cuenta_id DROP NOT NULL;
ALTER TABLE facturas_proveedor DROP CONSTRAINT facturas_proveedor_check5;

CREATE INDEX ix_facturas_proveedor_sin_contabilizar ON facturas_proveedor (tenant_id, fecha)
    WHERE comprobante_id IS NULL AND estado NOT IN ('registrada', 'anulada');

-- ---------------------------------------------------------------------
-- Permisos y plantillas de rol
-- ---------------------------------------------------------------------
-- Renombres (no hay producción): la venta es de la facturación y los terceros son transversales.
INSERT INTO permisos (codigo, ambito) VALUES
    ('facturacion.ver', 'copropiedad'),
    ('facturacion.facturas.gestionar', 'copropiedad'),
    ('facturacion.items.gestionar', 'copropiedad'),
    ('terceros.gestionar', 'copropiedad');

UPDATE roles_permisos SET permiso = 'facturacion.facturas.gestionar'
WHERE permiso = 'tesoreria.facturas_venta.gestionar';
UPDATE roles_permisos SET permiso = 'terceros.gestionar'
WHERE permiso = 'contabilidad.terceros.gestionar';
DELETE FROM permisos WHERE codigo IN ('tesoreria.facturas_venta.gestionar',
                                      'contabilidad.terceros.gestionar');

INSERT INTO roles (id, tenant_id, codigo, nombre, ambito, es_plantilla) VALUES
    (uuid_rol_plantilla('facturador'), NULL, 'facturador', 'Facturador', 'copropiedad', TRUE);

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('facturador'), 'copropiedad', p, NULL
FROM unnest(ARRAY[
    'archivos.ver', 'archivos.subir', 'personas.ver', 'personas.editar', 'terceros.gestionar',
    'facturacion.ver', 'facturacion.facturas.gestionar', 'facturacion.items.gestionar',
    'facturacion.emitir', 'facturacion.recibir']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r,
     unnest(ARRAY['facturacion.ver', 'facturacion.items.gestionar']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'facturacion.ver', NULL);

INSERT INTO permisos (codigo, ambito) VALUES ('facturacion.exportar', 'copropiedad');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'facturacion.exportar', NULL
FROM unnest(ARRAY['contador', 'administrador', 'facturador', 'revisor_fiscal']) AS r;

-- ---------------------------------------------------------------------
-- Consumo de documentos electrónicos por entidad (cobro del módulo, plan 19)
-- ---------------------------------------------------------------------
-- Cruza tenants: devuelve solo conteos (nunca datos de los documentos). Cuenta lo transmitido
-- (validado, enviado o en contingencia) creado en el rango; los rechazados no.
CREATE FUNCTION consumo_documentos_electronicos(p_desde DATE, p_hasta DATE)
RETURNS TABLE (tenant_id UUID, nombre TEXT, tipo TEXT, cantidad BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT d.tenant_id, e.nombre, d.tipo, count(*)
    FROM documentos_electronicos d
    JOIN entidades e ON e.id = d.tenant_id
    WHERE d.estado IN ('validado', 'enviado', 'contingencia')
      AND d.creado_en >= p_desde::timestamptz
      AND d.creado_en < (p_hasta + 1)::timestamptz
    GROUP BY d.tenant_id, e.nombre, d.tipo
    ORDER BY normalizar(e.nombre), d.tenant_id, d.tipo
$$;
REVOKE ALL ON FUNCTION consumo_documentos_electronicos(date, date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION consumo_documentos_electronicos(date, date) TO dad_app;
