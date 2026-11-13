-- Cartera de la PH, fase 5 (plan 14, ADR 0019): intereses de mora con la tasa de referencia,
-- acuerdos de pago y certificados (paz y salvo y deuda) verificables.
--
-- Garantías de la base de datos:
--   1. Las tasas de referencia de una modalidad no se solapan y no se modifican.
--   2. Una causación de intereses vigente por periodo; su detalle no se modifica.
--   3. Los intereses son obligaciones de origen `interes` (una por bien y causación): nunca
--      generan intereses (no hay anatocismo).
--   4. Un acuerdo de pago lo aprueba otra cuenta, y un bien tiene a lo sumo uno activo.
--   5. Del certificado solo se guarda el SHA-256 de su código de verificación.

-- ---------------------------------------------------------------------
-- Tasas de referencia (plataforma): interés bancario corriente certificado
-- ---------------------------------------------------------------------
CREATE TABLE tasas_referencia (
    id                   UUID         PRIMARY KEY,
    modalidad            TEXT         NOT NULL DEFAULT 'consumo_ordinario'
                         CHECK (modalidad IN ('consumo_ordinario')),
    vigente_desde        DATE         NOT NULL,
    vigente_hasta        DATE         NOT NULL CHECK (vigente_hasta >= vigente_desde),
    -- Efectiva anual, en %.
    tasa_efectiva_anual  NUMERIC(7,4) NOT NULL CHECK (tasa_efectiva_anual > 0
                                                       AND tasa_efectiva_anual <= 200),
    resolucion           TEXT         NOT NULL CHECK (char_length(resolucion) BETWEEN 1 AND 120),
    fuente               TEXT         NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    creada_por           UUID,
    creada_en            TIMESTAMPTZ  NOT NULL,
    EXCLUDE USING gist (modalidad WITH =,
                        daterange(vigente_desde, vigente_hasta, '[]') WITH &&)
);
ALTER TABLE tasas_referencia ENABLE ROW LEVEL SECURITY;
CREATE POLICY ver ON tasas_referencia FOR SELECT USING (TRUE);
CREATE POLICY crear ON tasas_referencia FOR INSERT WITH CHECK (app_plataforma());
GRANT SELECT, INSERT ON tasas_referencia TO dad_app;
CREATE TRIGGER tasas_referencia_solo_insercion BEFORE UPDATE OR DELETE ON tasas_referencia
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

-- ---------------------------------------------------------------------
-- Causación de intereses de mora
-- ---------------------------------------------------------------------
CREATE TABLE causaciones_mora (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    periodo           DATE          NOT NULL CHECK (extract(day FROM periodo) = 1),
    -- Último día causado (inclusive).
    hasta             DATE          NOT NULL,
    estado            TEXT          NOT NULL DEFAULT 'contabilizada'
                      CHECK (estado IN ('contabilizada', 'anulada')),
    total             NUMERIC(18,2) NOT NULL CHECK (total > 0),
    comprobante_id    UUID          NOT NULL,
    motivo_anulacion  TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    causada_por       UUID,
    causada_en        TIMESTAMPTZ   NOT NULL,
    anulada_en        TIMESTAMPTZ,
    CHECK ((estado = 'anulada') = (motivo_anulacion IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id) REFERENCES comprobantes (tenant_id, id)
);
CREATE UNIQUE INDEX ux_causacion_periodo ON causaciones_mora (tenant_id, periodo)
    WHERE estado = 'contabilizada';

ALTER TABLE obligaciones
    DROP CONSTRAINT obligaciones_origen_check,
    ADD CONSTRAINT obligaciones_origen_check CHECK (origen IN (
        'liquidacion', 'cargo', 'saldo_inicial', 'manual', 'interes')),
    ADD COLUMN causacion_id UUID,
    ADD CONSTRAINT ck_obligacion_causacion CHECK ((origen = 'interes') = (causacion_id IS NOT NULL)),
    ADD CONSTRAINT fk_obligacion_causacion
        FOREIGN KEY (tenant_id, causacion_id) REFERENCES causaciones_mora (tenant_id, id);
CREATE UNIQUE INDEX ux_obligacion_interes ON obligaciones (tenant_id, bien_privado_id, causacion_id)
    WHERE origen = 'interes' AND estado = 'vigente';

-- Trazabilidad que pide el CTCP: base, días y tasa de cada tramo.
CREATE TABLE causaciones_interes (
    id                     UUID          PRIMARY KEY,
    tenant_id              UUID          NOT NULL REFERENCES copropiedades (id),
    causacion_id           UUID          NOT NULL,
    -- Obligación de capital que generó el interés.
    obligacion_id          UUID          NOT NULL,
    -- Obligación de intereses donde quedó.
    obligacion_interes_id  UUID          NOT NULL,
    desde                  DATE          NOT NULL,
    hasta                  DATE          NOT NULL CHECK (hasta >= desde),
    dias                   INTEGER       NOT NULL CHECK (dias > 0),
    base                   NUMERIC(18,2) NOT NULL CHECK (base > 0),
    -- Tasa de mora aplicada (efectiva anual, %) y la de referencia de donde salió.
    tasa_efectiva_anual    NUMERIC(7,4)  NOT NULL CHECK (tasa_efectiva_anual > 0),
    tasa_referencia_id     UUID          NOT NULL REFERENCES tasas_referencia (id),
    valor                  NUMERIC(18,2) NOT NULL CHECK (valor >= 0),
    FOREIGN KEY (tenant_id, causacion_id)          REFERENCES causaciones_mora (tenant_id, id),
    FOREIGN KEY (tenant_id, obligacion_id)         REFERENCES obligaciones (tenant_id, id),
    FOREIGN KEY (tenant_id, obligacion_interes_id) REFERENCES obligaciones (tenant_id, id)
);
CREATE INDEX ix_causaciones_interes_obligacion ON causaciones_interes (tenant_id, obligacion_id, hasta);
CREATE INDEX ix_causaciones_interes_causacion ON causaciones_interes (tenant_id, causacion_id);

-- ---------------------------------------------------------------------
-- Acuerdos de pago
-- ---------------------------------------------------------------------
CREATE TABLE acuerdos_pago (
    id                 UUID          PRIMARY KEY,
    tenant_id          UUID          NOT NULL REFERENCES copropiedades (id),
    bien_privado_id    UUID          NOT NULL,
    deudor_persona_id  UUID          NOT NULL,
    -- Desde esta fecha la mora de lo que se debía queda suspendida mientras se cumpla.
    fecha              DATE          NOT NULL,
    valor              NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    numero_cuotas      SMALLINT      NOT NULL CHECK (numero_cuotas BETWEEN 1 AND 60),
    estado             TEXT          NOT NULL DEFAULT 'propuesto'
                       CHECK (estado IN ('propuesto', 'vigente', 'cumplido', 'incumplido',
                                         'anulado')),
    observaciones      TEXT          CHECK (char_length(observaciones) <= 500),
    creado_por         UUID          NOT NULL,
    creado_en          TIMESTAMPTZ   NOT NULL,
    aprobado_por       UUID,
    aprobado_en        TIMESTAMPTZ,
    cerrado_en         TIMESTAMPTZ,
    -- Segregación de funciones: lo aprueba otra cuenta.
    CHECK (aprobado_por IS NULL OR aprobado_por <> creado_por),
    CHECK ((estado = 'propuesto') = (aprobado_por IS NULL) OR estado = 'anulado'),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)   REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, deudor_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE UNIQUE INDEX ux_acuerdo_activo ON acuerdos_pago (tenant_id, bien_privado_id)
    WHERE estado IN ('propuesto', 'vigente');

CREATE TABLE cuotas_acuerdo (
    tenant_id    UUID          NOT NULL REFERENCES copropiedades (id),
    acuerdo_id   UUID          NOT NULL,
    numero       SMALLINT      NOT NULL CHECK (numero > 0),
    vencimiento  DATE          NOT NULL,
    valor        NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    PRIMARY KEY (acuerdo_id, numero),
    FOREIGN KEY (tenant_id, acuerdo_id) REFERENCES acuerdos_pago (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Certificados (paz y salvo y deuda, Ley 675 art. 48)
-- ---------------------------------------------------------------------
CREATE TABLE certificados_emitidos (
    id               UUID          PRIMARY KEY,
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    tipo             TEXT          NOT NULL CHECK (tipo IN ('paz_y_salvo', 'deuda')),
    bien_privado_id  UUID          NOT NULL,
    corte            DATE          NOT NULL,
    saldo            NUMERIC(18,2) NOT NULL CHECK (saldo >= 0),
    vencido          NUMERIC(18,2) NOT NULL CHECK (vencido >= 0),
    -- SHA-256 (hex) del código de verificación impreso; el código no se guarda.
    codigo_hash      TEXT          NOT NULL CHECK (codigo_hash ~ '^[0-9a-f]{64}$'),
    -- SHA-256 del PDF entregado.
    pdf_sha256       TEXT          NOT NULL CHECK (pdf_sha256 ~ '^[0-9a-f]{64}$'),
    emitido_por      UUID,
    emitido_en       TIMESTAMPTZ   NOT NULL,
    CHECK (tipo <> 'paz_y_salvo' OR saldo = 0),
    UNIQUE (tenant_id, codigo_hash),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);
CREATE INDEX ix_certificados_bien ON certificados_emitidos (tenant_id, bien_privado_id, emitido_en DESC);

-- ---------------------------------------------------------------------
-- Solo inserción, RLS y privilegios
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['causaciones_interes', 'cuotas_acuerdo', 'certificados_emitidos'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t || '_solo_insercion', t);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t || '_sin_truncate', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['causaciones_mora', 'causaciones_interes', 'acuerdos_pago',
                             'cuotas_acuerdo', 'certificados_emitidos'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON causaciones_mora, acuerdos_pago TO dad_app;
GRANT SELECT, INSERT ON causaciones_interes, cuotas_acuerdo, certificados_emitidos TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES ('cartera.certificados.emitir', 'copropiedad');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'cartera.certificados.emitir', NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r;
