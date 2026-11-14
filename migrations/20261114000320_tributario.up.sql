-- Tributario y documentos electrónicos (plan 15, ADR 0020). Genérico: sirve a cualquier entidad.
--
-- Ninguna tarifa, base ni formato vive en el código: son catálogos de plataforma versionados
-- (vigencia + fuente). El sistema **prepara**; presentar y firmar las declaraciones lo hace el
-- responsable tributario.
--
-- Garantías de la base de datos:
--   1. Un perfil tributario confirmado no se modifica (cada cambio es una versión nueva).
--   2. Un documento de negocio produce un solo documento electrónico de cada tipo (doble envío →
--      el mismo documento y el mismo CUFE).
--   3. Un número de una resolución de numeración no se usa dos veces; un CUFE tampoco.
--   4. Los eventos de los documentos electrónicos y de RADIAN son append-only.
--   5. Una factura recibida no se carga dos veces (CUFE y SHA-256 del archivo).

-- ---------------------------------------------------------------------
-- Módulo vendible
-- ---------------------------------------------------------------------
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'facturacion_electronica', 'bienes_comunes',
    'control_acceso', 'asambleas', 'gestion_administrativa', 'publicidad'));

-- XML de las facturas y documentos electrónicos (solo por el camino de la facturación).
ALTER TABLE archivos DROP CONSTRAINT archivos_tipo_mime_check;
ALTER TABLE archivos ADD CONSTRAINT archivos_tipo_mime_check CHECK (tipo_mime IN (
    'application/pdf', 'image/jpeg', 'image/png', 'image/webp', 'image/heic', 'text/csv',
    'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 'application/xml'));

-- ---------------------------------------------------------------------
-- Fase 1 · Catálogos de plataforma (IVA e ICA; UVT y retenciones vienen del plan 13)
-- ---------------------------------------------------------------------
CREATE TABLE tarifas_iva (
    id             UUID          PRIMARY KEY,
    codigo         TEXT          NOT NULL CHECK (codigo ~ '^[a-z][a-z0-9_]{1,59}$'),
    nombre         TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 3 AND 200),
    tratamiento    TEXT          NOT NULL CHECK (tratamiento IN (
                       'gravado', 'exento', 'excluido', 'no_sujeto')),
    -- Porcentaje; solo `gravado` lleva tarifa positiva.
    tarifa         NUMERIC(5,2)  NOT NULL CHECK (tarifa >= 0 AND tarifa <= 100),
    fuente         TEXT          NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    vigente_desde  DATE          NOT NULL,
    vigente_hasta  DATE          CHECK (vigente_hasta >= vigente_desde),
    CHECK ((tratamiento = 'gravado') = (tarifa > 0)),
    UNIQUE (codigo, vigente_desde)
);

CREATE TABLE tarifas_ica (
    id              UUID         PRIMARY KEY,
    municipio_dane  TEXT         NOT NULL CHECK (municipio_dane ~ '^[0-9]{5}$'),
    -- Código CIIU de la actividad.
    actividad       TEXT         NOT NULL CHECK (actividad ~ '^[0-9]{4}$'),
    nombre          TEXT         NOT NULL CHECK (char_length(nombre) BETWEEN 3 AND 200),
    tarifa_por_mil  NUMERIC(6,3) NOT NULL CHECK (tarifa_por_mil > 0 AND tarifa_por_mil <= 100),
    fuente          TEXT         NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    vigente_desde   DATE         NOT NULL,
    vigente_hasta   DATE         CHECK (vigente_hasta >= vigente_desde),
    UNIQUE (municipio_dane, actividad, vigente_desde)
);

-- Información exógena: formato por año gravable y versión, con su mapeo cuenta → concepto.
CREATE TABLE formatos_exogena (
    id             UUID          PRIMARY KEY,
    anio           SMALLINT      NOT NULL CHECK (anio BETWEEN 2000 AND 2999),
    codigo         TEXT          NOT NULL CHECK (codigo ~ '^[0-9]{4}$'),
    version        INTEGER       NOT NULL CHECK (version > 0),
    nombre         TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 3 AND 200),
    resolucion     TEXT          NOT NULL CHECK (char_length(resolucion) BETWEEN 3 AND 200),
    -- Por debajo de esta cuantía los terceros se agregan en el NIT 222222222 (cuantías menores).
    cuantia_menor  NUMERIC(18,2) NOT NULL CHECK (cuantia_menor >= 0),
    creado_en      TIMESTAMPTZ   NOT NULL,
    UNIQUE (anio, codigo, version)
);

CREATE TABLE campos_formato_exogena (
    formato_id      UUID NOT NULL REFERENCES formatos_exogena (id),
    -- Concepto DIAN (p. ej. 5002 honorarios).
    concepto        TEXT NOT NULL CHECK (concepto ~ '^[0-9]{4}$'),
    -- Prefijo de las cuentas que suman al concepto (1 a 10 dígitos).
    cuenta_prefijo  TEXT NOT NULL CHECK (cuenta_prefijo ~ '^[1-9][0-9]{0,9}$'),
    -- Columna del formato (`pago_deducible`, `iva_mayor_valor`, `retencion_renta`…).
    columna         TEXT NOT NULL CHECK (columna ~ '^[a-z][a-z0-9_]{1,59}$'),
    -- Qué lado del movimiento suma: débitos, créditos o el neto.
    lado            TEXT NOT NULL CHECK (lado IN ('debito', 'credito', 'neto')),
    PRIMARY KEY (formato_id, concepto, cuenta_prefijo, columna)
);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['tarifas_iva', 'tarifas_ica', 'formatos_exogena',
                             'campos_formato_exogena'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY ver ON %I FOR SELECT USING (TRUE)', t);
        EXECUTE format('CREATE POLICY crear ON %I FOR INSERT WITH CHECK (app_plataforma())', t);
        EXECUTE format('CREATE POLICY editar ON %I FOR UPDATE USING (app_plataforma()) '
                       'WITH CHECK (app_plataforma())', t);
    END LOOP;
END $$;
GRANT SELECT, INSERT, UPDATE ON tarifas_iva, tarifas_ica, formatos_exogena TO dad_app;
GRANT SELECT, INSERT ON campos_formato_exogena TO dad_app;

-- ---------------------------------------------------------------------
-- Fase 1 · Perfil tributario de la entidad (versionado; sin confirmar no hay automatismos)
-- ---------------------------------------------------------------------
CREATE TABLE perfiles_tributarios (
    id                      UUID        PRIMARY KEY,
    tenant_id               UUID        NOT NULL REFERENCES entidades (id),
    version                 INTEGER     NOT NULL CHECK (version > 0),
    vigente_desde           DATE        NOT NULL,
    estado                  TEXT        NOT NULL DEFAULT 'borrador'
                            CHECK (estado IN ('borrador', 'confirmado')),
    -- Responsabilidades del RUT (`O-13`, `O-15`, `O-47`, `O-48`, `O-49`, `R-99-PN`).
    responsabilidades       TEXT[]      NOT NULL DEFAULT '{}',
    agente_retenedor        BOOLEAN     NOT NULL,
    responsable_iva         BOOLEAN     NOT NULL,
    gran_contribuyente      BOOLEAN     NOT NULL,
    autorretenedor          BOOLEAN     NOT NULL,
    contribuyente_renta     BOOLEAN     NOT NULL,
    facturador_electronico  BOOLEAN     NOT NULL,
    obligado_rub            BOOLEAN     NOT NULL,
    -- Códigos DANE de los municipios donde declara ICA y CIIU de sus actividades.
    municipios_ica          TEXT[]      NOT NULL DEFAULT '{}',
    actividades_ica         TEXT[]      NOT NULL DEFAULT '{}',
    canal_facturacion       TEXT        NOT NULL DEFAULT 'ninguno'
                            CHECK (canal_facturacion IN ('ninguno', 'proveedor', 'propio')),
    -- RUT u otro soporte.
    soporte_archivo_id      UUID,
    observaciones           TEXT        CHECK (char_length(observaciones) <= 500),
    creado_por              UUID,
    creado_en               TIMESTAMPTZ NOT NULL,
    confirmado_por          UUID,
    confirmado_en           TIMESTAMPTZ,
    CHECK ((estado = 'confirmado') = (confirmado_por IS NOT NULL AND confirmado_en IS NOT NULL)),
    UNIQUE (tenant_id, version),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);

-- Un perfil confirmado es la decisión del contador: no se modifica.
CREATE FUNCTION exigir_perfil_borrador() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.estado = 'confirmado' THEN
        RAISE EXCEPTION 'un perfil tributario confirmado no se modifica: cree una versión nueva'
            USING ERRCODE = 'V0002';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_perfil_solo_borrador BEFORE UPDATE OR DELETE ON perfiles_tributarios
    FOR EACH ROW EXECUTE FUNCTION exigir_perfil_borrador();

-- ---------------------------------------------------------------------
-- Fase 2 · Clasificación tributaria de lo que se cobra o se vende (solo `alquiler_servicio` y
-- `venta` pueden llevar IVA)
-- ---------------------------------------------------------------------
ALTER TABLE lineas_factura_venta
    ADD COLUMN clasificacion TEXT NOT NULL DEFAULT 'otra' CHECK (clasificacion IN (
        'expensa', 'cuota_extraordinaria', 'uso_sin_cobro', 'alquiler_servicio', 'venta', 'otra')),
    ADD CONSTRAINT ck_linea_venta_iva
        CHECK (tarifa_iva = 0 OR clasificacion IN ('alquiler_servicio', 'venta'));

ALTER TABLE conceptos_cobro
    ADD COLUMN clasificacion_tributaria TEXT NOT NULL DEFAULT 'expensa' CHECK (
        clasificacion_tributaria IN ('expensa', 'cuota_extraordinaria', 'uso_sin_cobro',
                                     'alquiler_servicio', 'venta', 'otra'));
UPDATE conceptos_cobro SET clasificacion_tributaria = CASE tipo
    WHEN 'extraordinaria' THEN 'cuota_extraordinaria'
    WHEN 'uso_bien_comun' THEN 'alquiler_servicio'
    WHEN 'multa' THEN 'otra'
    WHEN 'dano' THEN 'otra'
    WHEN 'interes_mora' THEN 'otra'
    WHEN 'otro' THEN 'otra'
    ELSE 'expensa' END;

-- ---------------------------------------------------------------------
-- Fase 3 · Resoluciones de numeración y documentos electrónicos
-- ---------------------------------------------------------------------
CREATE TABLE resoluciones_numeracion (
    id                 UUID        PRIMARY KEY,
    tenant_id          UUID        NOT NULL REFERENCES entidades (id),
    tipo_documento     TEXT        NOT NULL CHECK (tipo_documento IN (
                           'factura', 'documento_soporte')),
    prefijo            TEXT        NOT NULL CHECK (prefijo ~ '^[A-Z0-9]{0,4}$'),
    desde              BIGINT      NOT NULL CHECK (desde > 0),
    hasta              BIGINT      NOT NULL CHECK (hasta >= desde),
    numero_resolucion  TEXT        NOT NULL CHECK (char_length(numero_resolucion) BETWEEN 1 AND 40),
    fecha_resolucion   DATE        NOT NULL,
    vigente_desde      DATE        NOT NULL,
    vigente_hasta      DATE        NOT NULL CHECK (vigente_hasta >= vigente_desde),
    -- Clave técnica de la resolución (la entrega la DIAN; la usa el CUFE).
    clave_tecnica      TEXT        CHECK (char_length(clave_tecnica) <= 200),
    activa             BOOLEAN     NOT NULL DEFAULT TRUE,
    creada_en          TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, tipo_documento, prefijo, desde),
    UNIQUE (tenant_id, id)
);
COMMENT ON COLUMN resoluciones_numeracion.clave_tecnica IS
    'secreto: clave técnica de la DIAN; no se expone en la API ni en los logs';

CREATE TABLE documentos_electronicos (
    id                       UUID        PRIMARY KEY,
    tenant_id                UUID        NOT NULL REFERENCES entidades (id),
    tipo                     TEXT        NOT NULL CHECK (tipo IN (
                                 'factura', 'nota_credito', 'nota_debito', 'documento_soporte',
                                 'nota_ajuste_ds')),
    -- Documento de negocio que lo origina (`ventas.factura`, `cxp.factura`…).
    origen_tipo              TEXT        NOT NULL CHECK (origen_tipo ~ '^[a-z][a-z0-9_.]{2,79}$'),
    origen_id                UUID        NOT NULL,
    resolucion_id            UUID,
    prefijo                  TEXT        NOT NULL DEFAULT '',
    numero                   BIGINT      CHECK (numero > 0),
    -- Documento al que se refiere una nota.
    referencia_id            UUID,
    cufe                     TEXT        CHECK (char_length(cufe) BETWEEN 10 AND 200),
    estado                   TEXT        NOT NULL DEFAULT 'por_enviar' CHECK (estado IN (
                                 'por_enviar', 'enviado', 'validado', 'rechazado',
                                 'contingencia')),
    -- Adaptador que lo transmitió (`simulado`, el del proveedor elegido o `propio`).
    proveedor                TEXT        NOT NULL CHECK (proveedor ~ '^[a-z][a-z0-9_]{1,39}$'),
    id_externo               TEXT        CHECK (char_length(id_externo) <= 200),
    xml_archivo_id           UUID,
    -- Respuesta de la DIAN o del proveedor (mensaje; el XML de respuesta va en archivos).
    respuesta                TEXT        CHECK (char_length(respuesta) <= 4000),
    intentos                 INTEGER     NOT NULL DEFAULT 0 CHECK (intentos >= 0),
    total                    NUMERIC(18,2) NOT NULL CHECK (total >= 0),
    creado_en                TIMESTAMPTZ NOT NULL,
    enviado_en               TIMESTAMPTZ,
    validado_en              TIMESTAMPTZ,
    CHECK ((estado = 'validado') = (validado_en IS NOT NULL)),
    CHECK (estado <> 'validado' OR cufe IS NOT NULL),
    UNIQUE (tenant_id, tipo, origen_tipo, origen_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, resolucion_id)  REFERENCES resoluciones_numeracion (tenant_id, id),
    FOREIGN KEY (tenant_id, referencia_id)  REFERENCES documentos_electronicos (tenant_id, id),
    FOREIGN KEY (tenant_id, xml_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_documento_numero ON documentos_electronicos (tenant_id, tipo, prefijo, numero)
    WHERE numero IS NOT NULL;
CREATE UNIQUE INDEX ux_documento_cufe ON documentos_electronicos (cufe) WHERE cufe IS NOT NULL;
CREATE INDEX ix_documentos_estado ON documentos_electronicos (tenant_id, estado, creado_en);

CREATE TABLE eventos_documento_electronico (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES entidades (id),
    documento_id  UUID        NOT NULL,
    tipo          TEXT        NOT NULL CHECK (tipo ~ '^[a-z][a-z0-9_]{1,39}$'),
    detalle       TEXT        CHECK (char_length(detalle) <= 4000),
    ocurrido_en   TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, documento_id) REFERENCES documentos_electronicos (tenant_id, id)
);
CREATE INDEX ix_eventos_documento ON eventos_documento_electronico (tenant_id, documento_id, ocurrido_en);

-- ---------------------------------------------------------------------
-- Fase 4 · Facturas recibidas (buzón) y eventos RADIAN
-- ---------------------------------------------------------------------
CREATE TABLE facturas_recibidas (
    id                    UUID          PRIMARY KEY,
    tenant_id             UUID          NOT NULL REFERENCES entidades (id),
    archivo_id            UUID          NOT NULL,
    sha256                TEXT          NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    emisor_nit            TEXT          NOT NULL CHECK (emisor_nit ~ '^[0-9]{5,15}$'),
    emisor_nombre         TEXT          NOT NULL CHECK (char_length(emisor_nombre) BETWEEN 1 AND 300),
    prefijo               TEXT          NOT NULL DEFAULT '' CHECK (char_length(prefijo) <= 10),
    numero                TEXT          NOT NULL CHECK (char_length(numero) BETWEEN 1 AND 40),
    fecha                 DATE          NOT NULL,
    vencimiento           DATE,
    cufe                  TEXT          NOT NULL CHECK (char_length(cufe) BETWEEN 10 AND 200),
    subtotal              NUMERIC(18,2) NOT NULL CHECK (subtotal >= 0),
    iva                   NUMERIC(18,2) NOT NULL CHECK (iva >= 0),
    total                 NUMERIC(18,2) NOT NULL CHECK (total >= 0),
    estado                TEXT          NOT NULL DEFAULT 'pendiente'
                          CHECK (estado IN ('pendiente', 'registrada', 'rechazada')),
    -- Proveedor de la entidad con ese NIT, si existe.
    persona_id            UUID,
    factura_proveedor_id  UUID,
    motivo_rechazo        TEXT          CHECK (char_length(motivo_rechazo) BETWEEN 5 AND 500),
    recibida_por          UUID,
    recibida_en           TIMESTAMPTZ   NOT NULL,
    CHECK ((estado = 'registrada') = (factura_proveedor_id IS NOT NULL)),
    CHECK ((estado = 'rechazada') = (motivo_rechazo IS NOT NULL)),
    UNIQUE (tenant_id, cufe),
    UNIQUE (tenant_id, sha256),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id)           REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)           REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, factura_proveedor_id) REFERENCES facturas_proveedor (tenant_id, id)
);
CREATE INDEX ix_facturas_recibidas_estado ON facturas_recibidas (tenant_id, estado, recibida_en DESC);

CREATE TABLE eventos_radian (
    id                   UUID        PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES entidades (id),
    factura_recibida_id  UUID        NOT NULL,
    tipo                 TEXT        NOT NULL CHECK (tipo IN (
                             'acuse', 'recibo_bien', 'aceptacion', 'reclamo')),
    estado               TEXT        NOT NULL CHECK (estado IN ('enviado', 'rechazado')),
    id_externo           TEXT        CHECK (char_length(id_externo) <= 200),
    motivo               TEXT        CHECK (char_length(motivo) <= 500),
    respuesta            TEXT        CHECK (char_length(respuesta) <= 4000),
    creado_por           UUID,
    creado_en            TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, factura_recibida_id, tipo),
    FOREIGN KEY (tenant_id, factura_recibida_id) REFERENCES facturas_recibidas (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Fase 5 · Exógena y RUB
-- ---------------------------------------------------------------------
CREATE TABLE generaciones_exogena (
    id            UUID          PRIMARY KEY,
    tenant_id     UUID          NOT NULL REFERENCES entidades (id),
    formato_id    UUID          NOT NULL REFERENCES formatos_exogena (id),
    anio          SMALLINT      NOT NULL,
    version       INTEGER       NOT NULL,
    filas         INTEGER       NOT NULL CHECK (filas >= 0),
    total         NUMERIC(18,2) NOT NULL,
    errores       INTEGER       NOT NULL CHECK (errores >= 0),
    sha256        TEXT          NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    archivo_id    UUID          NOT NULL,
    generada_por  UUID,
    generada_en   TIMESTAMPTZ   NOT NULL,
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_generaciones_exogena ON generaciones_exogena (tenant_id, anio, generada_en DESC);

-- Beneficiarios finales (RUB). Un propietario no es beneficiario final de la PH por serlo.
CREATE TABLE beneficiarios_finales (
    id                  UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL REFERENCES entidades (id),
    persona_id          UUID         NOT NULL,
    criterio            TEXT         NOT NULL CHECK (criterio IN ('propiedad', 'control', 'beneficio')),
    porcentaje          NUMERIC(7,4) CHECK (porcentaje > 0 AND porcentaje <= 100),
    soporte_archivo_id  UUID,
    vigente_desde       DATE         NOT NULL,
    vigente_hasta       DATE         CHECK (vigente_hasta >= vigente_desde),
    creado_por          UUID,
    creado_en           TIMESTAMPTZ  NOT NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)         REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_beneficiarios_persona ON beneficiarios_finales (tenant_id, persona_id);

-- ---------------------------------------------------------------------
-- Solo inserción, RLS y privilegios
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['eventos_documento_electronico', 'eventos_radian',
                             'generaciones_exogena'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t || '_solo_insercion', t);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t || '_sin_truncate', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['perfiles_tributarios', 'resoluciones_numeracion',
                             'documentos_electronicos', 'eventos_documento_electronico',
                             'facturas_recibidas', 'eventos_radian', 'generaciones_exogena',
                             'beneficiarios_finales'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON perfiles_tributarios, resoluciones_numeracion,
    documentos_electronicos, facturas_recibidas, beneficiarios_finales TO dad_app;
GRANT SELECT, INSERT ON eventos_documento_electronico, eventos_radian, generaciones_exogena
    TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('tributario.configurar', 'copropiedad'),
    ('tributario.perfil.confirmar', 'copropiedad'),
    ('tributario.preparar', 'copropiedad'),
    ('facturacion.emitir', 'copropiedad'),
    ('facturacion.recibir', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r,
     unnest(ARRAY['tributario.configurar', 'tributario.perfil.confirmar', 'tributario.preparar',
                  'facturacion.emitir', 'facturacion.recibir']) AS p;
