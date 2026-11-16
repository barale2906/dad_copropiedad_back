-- Plan 19 (ADR 0024): la plataforma cobra a sus clientes lo que tienen contratado.
--
-- Catálogo comercial, tarifas, suscripciones, cortes y mediciones son tablas de PLATAFORMA (solo
-- las ve el equipo de la plataforma; la entidad ve lo suyo por su caso de uso). Las facturas son
-- facturas normales del tenant de la entidad de la plataforma (`entidades.es_plataforma`).
-- La medición entre tenants va por una función SECURITY DEFINER que devuelve solo números.

CREATE TABLE productos_plataforma (
    id                   UUID        PRIMARY KEY,
    codigo               TEXT        NOT NULL UNIQUE CHECK (codigo ~ '^[a-z][a-z0-9_]{1,39}$'),
    nombre               TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    -- Plantilla del texto de la factura: {producto}, {periodo}, {cantidad}, {incluidas}.
    descripcion_factura  TEXT        NOT NULL CHECK (char_length(descripcion_factura) BETWEEN 1 AND 300),
    modulo               TEXT,
    -- Ítem de facturación del tenant de la plataforma (cuenta e IVA los define su contador).
    item_facturacion_id  UUID,
    activo               BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en            TIMESTAMPTZ NOT NULL
);

CREATE TABLE tarifas_producto (
    id               UUID          PRIMARY KEY,
    producto_id      UUID          NOT NULL REFERENCES productos_plataforma (id),
    modelo           TEXT          NOT NULL CHECK (modelo IN ('plana', 'por_unidad', 'por_paquete',
                                       'por_consumo', 'unica')),
    valor            NUMERIC(14,2) NOT NULL CHECK (valor >= 0),
    incluidas        INTEGER       CHECK (incluidas >= 0),
    valor_excedente  NUMERIC(14,2) CHECK (valor_excedente >= 0),
    metrica          TEXT          CHECK (metrica ~ '^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$'),
    vigente_desde    DATE          NOT NULL,
    creada_por       UUID          REFERENCES cuentas (id),
    creada_en        TIMESTAMPTZ   NOT NULL,
    UNIQUE (producto_id, vigente_desde),
    CHECK ((modelo IN ('por_unidad', 'por_paquete', 'por_consumo')) = (metrica IS NOT NULL)),
    CHECK ((modelo = 'por_paquete') = (incluidas IS NOT NULL AND valor_excedente IS NOT NULL))
);
-- Versionadas: una tarifa nueva es una fila nueva.
CREATE TRIGGER trg_tarifas_producto_solo_insercion BEFORE UPDATE OR DELETE ON tarifas_producto
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

CREATE TABLE suscripciones (
    id                       UUID        PRIMARY KEY,
    tenant_id                UUID        NOT NULL REFERENCES entidades (id),
    -- Quién paga: la misma entidad (los dos vacíos), otra entidad u otra organización.
    pagador_entidad_id       UUID        REFERENCES entidades (id),
    pagador_organizacion_id  UUID        REFERENCES organizaciones (id),
    ciclo_meses              SMALLINT    CHECK (ciclo_meses BETWEEN 1 AND 12),
    -- NULL en ciclo_meses = por evento (se corta a pedido).
    inicio                   DATE        NOT NULL,
    dias_prueba              SMALLINT    NOT NULL DEFAULT 0 CHECK (dias_prueba BETWEEN 0 AND 30),
    plazo_pago_dias          SMALLINT    NOT NULL DEFAULT 15 CHECK (plazo_pago_dias BETWEEN 0 AND 120),
    correo_facturacion       TEXT        NOT NULL CHECK (correo_facturacion = lower(correo_facturacion)
                                             AND correo_facturacion LIKE '%_@_%'),
    estado                   TEXT        NOT NULL DEFAULT 'activa'
                                         CHECK (estado IN ('activa', 'suspendida', 'terminada')),
    -- La plataforma la suspendió por mora: al pagar se reactiva la entidad.
    suspendida_por_mora      BOOLEAN     NOT NULL DEFAULT FALSE,
    terminada_en             DATE,
    creada_por               UUID        REFERENCES cuentas (id),
    creada_en                TIMESTAMPTZ NOT NULL,
    CHECK (pagador_entidad_id IS NULL OR pagador_organizacion_id IS NULL),
    CHECK ((estado = 'terminada') = (terminada_en IS NOT NULL))
);
COMMENT ON COLUMN suscripciones.correo_facturacion IS 'dato_personal: correo de facturación';
CREATE UNIQUE INDEX ux_suscripcion_vigente ON suscripciones (tenant_id) WHERE estado <> 'terminada';

CREATE TABLE lineas_suscripcion (
    id              UUID          PRIMARY KEY,
    suscripcion_id  UUID          NOT NULL REFERENCES suscripciones (id),
    producto_id     UUID          NOT NULL REFERENCES productos_plataforma (id),
    precio_pactado  NUMERIC(14,2) CHECK (precio_pactado >= 0),
    motivo_precio   TEXT          CHECK (char_length(motivo_precio) BETWEEN 1 AND 300),
    desde           DATE          NOT NULL,
    hasta           DATE          CHECK (hasta > desde),
    creada_en       TIMESTAMPTZ   NOT NULL,
    CHECK ((precio_pactado IS NULL) = (motivo_precio IS NULL))
);
CREATE INDEX ix_lineas_suscripcion ON lineas_suscripcion (suscripcion_id, desde);

CREATE TABLE cortes_facturacion (
    id                 UUID          PRIMARY KEY,
    -- Un corte es de una suscripción o de una liquidación de publicidad (un anunciante).
    suscripcion_id     UUID          REFERENCES suscripciones (id),
    liquidacion_id     UUID          REFERENCES liquidaciones (id),
    periodo_desde      DATE          NOT NULL,
    periodo_hasta      DATE          NOT NULL CHECK (periodo_hasta > periodo_desde),
    lineas             JSONB         NOT NULL,
    total              NUMERIC(16,2) NOT NULL CHECK (total >= 0),
    -- SHA-256 de las mediciones y las líneas (reproducible).
    huella             TEXT          NOT NULL CHECK (huella ~ '^[0-9a-f]{64}$'),
    estado             TEXT          NOT NULL CHECK (estado IN ('borrador', 'aprobado', 'facturado',
                                         'pagado', 'descartado')),
    motivo             TEXT          CHECK (char_length(motivo) BETWEEN 1 AND 500),
    aprobado_por       UUID          REFERENCES cuentas (id),
    aprobado_en        TIMESTAMPTZ,
    -- Factura del tenant de la plataforma, o la emitida por fuera mientras no esté habilitada.
    factura_venta_id   UUID,
    factura_externa    TEXT          CHECK (char_length(factura_externa) BETWEEN 1 AND 60),
    facturado_en       DATE,
    vencimiento        DATE,
    pagado_en          DATE,
    ultimo_aviso_en    DATE,
    creado_en          TIMESTAMPTZ   NOT NULL,
    CHECK ((suscripcion_id IS NULL) <> (liquidacion_id IS NULL)),
    CHECK (estado <> 'descartado' OR motivo IS NOT NULL),
    CHECK (estado NOT IN ('facturado', 'pagado')
           OR (facturado_en IS NOT NULL AND vencimiento IS NOT NULL
               AND (factura_venta_id IS NOT NULL OR factura_externa IS NOT NULL))),
    CHECK ((estado = 'pagado') = (pagado_en IS NOT NULL))
);
CREATE UNIQUE INDEX ux_corte_vigente_suscripcion ON cortes_facturacion (suscripcion_id, periodo_desde)
    WHERE estado <> 'descartado' AND suscripcion_id IS NOT NULL;
CREATE UNIQUE INDEX ux_corte_vigente_liquidacion ON cortes_facturacion (liquidacion_id)
    WHERE estado <> 'descartado' AND liquidacion_id IS NOT NULL;
CREATE INDEX ix_cortes_por_cobrar ON cortes_facturacion (vencimiento) WHERE estado = 'facturado';

CREATE TABLE mediciones_consumo (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES entidades (id),
    metrica        TEXT        NOT NULL,
    periodo_desde  DATE        NOT NULL,
    periodo_hasta  DATE        NOT NULL,
    cantidad       BIGINT      NOT NULL CHECK (cantidad >= 0),
    medida_en      TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, metrica, periodo_desde, periodo_hasta)
);
CREATE TRIGGER trg_mediciones_solo_insercion BEFORE UPDATE OR DELETE ON mediciones_consumo
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['productos_plataforma', 'tarifas_producto', 'suscripciones',
                             'lineas_suscripcion', 'cortes_facturacion', 'mediciones_consumo'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) '
                       'WITH CHECK (app_plataforma())', t);
    END LOOP;
END
$$;
GRANT SELECT, INSERT, UPDATE ON productos_plataforma, suscripciones, lineas_suscripcion,
    cortes_facturacion TO dad_app;
GRANT SELECT, INSERT ON tarifas_producto, mediciones_consumo TO dad_app;

-- Cantidad de una métrica de la entidad en el periodo [desde, hasta): solo números. Una métrica
-- desconocida es un error (se agrega aquí cuando una vertical la necesite).
CREATE FUNCTION medir_consumo(p_tenant UUID, p_metrica TEXT, p_desde DATE, p_hasta DATE)
RETURNS BIGINT
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT app_plataforma() THEN
        RAISE EXCEPTION 'medir_consumo exige una transacción de plataforma';
    END IF;
    RETURN CASE p_metrica
        WHEN 'ph.bienes_privados' THEN
            (SELECT count(*) FROM bienes_privados b WHERE b.tenant_id = p_tenant AND b.activo)
        WHEN 'facturacion.documentos' THEN
            (SELECT count(*) FROM documentos_electronicos d
             WHERE d.tenant_id = p_tenant AND d.estado IN ('validado', 'enviado', 'contingencia')
               AND d.creado_en >= p_desde::timestamptz AND d.creado_en < p_hasta::timestamptz)
        WHEN 'whatsapp.mensajes' THEN
            (SELECT count(*) FROM outbox o
             WHERE o.tenant_id = p_tenant AND o.canal = 'whatsapp'
               AND o.creado_en >= p_desde::timestamptz AND o.creado_en < p_hasta::timestamptz)
        WHEN 'usuarios.cuentas' THEN
            (SELECT count(DISTINCT a.cuenta_id) FROM asignaciones_rol a
             WHERE a.tenant_id = p_tenant AND a.vigente_desde < p_hasta::timestamptz
               AND (a.vigente_hasta IS NULL OR a.vigente_hasta > p_desde::timestamptz))
        ELSE NULL
    END;
END
$$;
REVOKE ALL ON FUNCTION medir_consumo(uuid, text, date, date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION medir_consumo(uuid, text, date, date) TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES
    ('plataforma.comercial.gestionar', 'plataforma'),
    ('plataforma.cobros.aprobar', 'plataforma'),
    ('plataforma.cobros.facturar', 'plataforma'),
    ('suscripcion.ver', 'copropiedad');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('superusuario'), 'plataforma', p, NULL
FROM unnest(ARRAY['plataforma.comercial.gestionar', 'plataforma.cobros.aprobar',
                  'plataforma.cobros.facturar']) AS p;
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('comercial'), 'plataforma', 'plataforma.comercial.gestionar', NULL),
    (uuid_rol_plantilla('facturacion'), 'plataforma', 'plataforma.cobros.aprobar', NULL),
    (uuid_rol_plantilla('facturacion'), 'plataforma', 'plataforma.cobros.facturar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'suscripcion.ver', NULL);
