-- Vertical contable de la propiedad horizontal (plan 14 fases 1–4, ADR 0019).
--
-- Se monta sobre lo que la PH ya tiene (bienes privados, cuadros de coeficientes, titulares,
-- preguntas de la asamblea, cargos de las reservas) y sobre el núcleo contable y la tesorería
-- (comprobantes, recibos de caja, aplicaciones de pago). Garantías de la base de datos:
--   1. Una obligación liquidada por (bien, concepto, periodo, cuota): reliquidar no duplica.
--   2. Una liquidación vigente por periodo.
--   3. Políticas de cartera y cuentas de cobro no se modifican (solo inserción).
--   4. El saldo de una obligación se reconstruye: valor − ajustes vigentes − aplicaciones
--      vigentes (no hay columna de saldo).
--   5. Una aplicación de pago va a una factura de venta o a una obligación, nunca a ambas.

-- ---------------------------------------------------------------------
-- Módulo vendible
-- ---------------------------------------------------------------------
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'bienes_comunes', 'control_acceso', 'asambleas',
    'gestion_administrativa', 'publicidad'));

-- ---------------------------------------------------------------------
-- Fase 1 · Datos de pago del bien, conceptos de cobro y políticas de cartera
-- ---------------------------------------------------------------------
ALTER TABLE bienes_privados
    -- Referencia numérica del recaudo (Asobancaria, pasarela, consignaciones).
    ADD COLUMN referencia_pago     TEXT CHECK (referencia_pago ~ '^[0-9]{1,24}$'),
    -- Quien paga (tenedor, art. 29); NULL = el propietario representante, que siempre es el
    -- deudor en el mayor.
    ADD COLUMN pagador_persona_id  UUID,
    ADD CONSTRAINT fk_bienes_privados_pagador
        FOREIGN KEY (tenant_id, pagador_persona_id) REFERENCES personas (tenant_id, id);
CREATE UNIQUE INDEX ux_bienes_referencia_pago ON bienes_privados (tenant_id, referencia_pago)
    WHERE referencia_pago IS NOT NULL;

CREATE TABLE modulos_contribucion (
    id           UUID        PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    nombre       TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 100),
    descripcion  TEXT        CHECK (char_length(descripcion) <= 300),
    activo       BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en    TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, nombre),
    UNIQUE (tenant_id, id)
);

-- Cada módulo de contribución tiene su propio cuadro (suma 1 dentro del módulo; solo los bienes
-- que contribuyen a él). El cuadro de la copropiedad no cambia.
ALTER TABLE cuadros_coeficientes
    ADD COLUMN modulo_id UUID,
    ADD CONSTRAINT ck_cuadro_modulo CHECK ((tipo = 'modulo_contribucion') = (modulo_id IS NOT NULL)),
    ADD CONSTRAINT fk_cuadro_modulo
        FOREIGN KEY (tenant_id, modulo_id) REFERENCES modulos_contribucion (tenant_id, id),
    DROP CONSTRAINT cuadros_coeficientes_tenant_id_tipo_version_key;
CREATE UNIQUE INDEX ux_cuadro_version ON cuadros_coeficientes
    (tenant_id, tipo, coalesce(modulo_id, '00000000-0000-0000-0000-000000000000'::uuid), version);
DROP INDEX ux_cuadro_vigente;
CREATE UNIQUE INDEX ux_cuadro_vigente ON cuadros_coeficientes
    (tenant_id, tipo, coalesce(modulo_id, '00000000-0000-0000-0000-000000000000'::uuid))
    WHERE estado = 'vigente';

CREATE TABLE conceptos_cobro (
    id                 UUID          PRIMARY KEY,
    tenant_id          UUID          NOT NULL REFERENCES copropiedades (id),
    codigo             TEXT          NOT NULL CHECK (codigo ~ '^[a-z][a-z0-9_]{1,39}$'),
    nombre             TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 100),
    tipo               TEXT          NOT NULL CHECK (tipo IN (
                           'ordinaria', 'fondo_imprevistos', 'extraordinaria', 'uso_bien_comun',
                           'multa', 'interes_mora', 'dano', 'otro')),
    criterio           TEXT          NOT NULL CHECK (criterio IN (
                           'coeficiente', 'modulo', 'valor_fijo', 'por_tipo_bien', 'area')),
    modulo_id          UUID,
    -- Valor mensual por bien (criterio `valor_fijo`).
    valor_fijo         NUMERIC(18,2) CHECK (valor_fijo > 0),
    genera_interes     BOOLEAN       NOT NULL DEFAULT FALSE,
    -- Concepto de las reglas `ph.*` del contador (P1): elige las cuentas de cartera e ingreso.
    concepto_contable  TEXT          NOT NULL CHECK (concepto_contable ~ '^[a-z][a-z0-9_]{0,59}$'),
    activo             BOOLEAN       NOT NULL DEFAULT TRUE,
    creado_en          TIMESTAMPTZ   NOT NULL,
    CHECK ((criterio = 'modulo') = (modulo_id IS NOT NULL)),
    CHECK ((criterio = 'valor_fijo') = (valor_fijo IS NOT NULL)),
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, modulo_id) REFERENCES modulos_contribucion (tenant_id, id)
);

-- Valor mensual por tipo de bien (criterio `por_tipo_bien`); los tipos sin tarifa no pagan.
CREATE TABLE tarifas_concepto_cobro (
    tenant_id    UUID          NOT NULL REFERENCES copropiedades (id),
    concepto_id  UUID          NOT NULL,
    tipo_bien    TEXT          NOT NULL CHECK (tipo_bien IN (
                     'apartamento', 'casa', 'local', 'oficina', 'consultorio', 'parqueadero',
                     'deposito', 'bodega', 'lote', 'finca', 'otro')),
    valor        NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    PRIMARY KEY (concepto_id, tipo_bien),
    FOREIGN KEY (tenant_id, concepto_id) REFERENCES conceptos_cobro (tenant_id, id)
);

-- Versionada: cada cambio es una versión nueva (con la pregunta de la asamblea que la aprobó).
CREATE TABLE politicas_cartera (
    id                         UUID          PRIMARY KEY,
    tenant_id                  UUID          NOT NULL REFERENCES copropiedades (id),
    version                    INTEGER       NOT NULL CHECK (version > 0),
    vigente_desde              DATE          NOT NULL,
    dia_vencimiento            SMALLINT      NOT NULL CHECK (dia_vencimiento BETWEEN 1 AND 28),
    -- Múltiplo al que se redondea cada cobro (1 = al peso, 100 = a la centena).
    redondeo                   NUMERIC(18,2) NOT NULL CHECK (redondeo >= 1),
    pronto_pago_porcentaje     NUMERIC(5,2)  CHECK (pronto_pago_porcentaje > 0
                                                    AND pronto_pago_porcentaje <= 100),
    pronto_pago_dia            SMALLINT      CHECK (pronto_pago_dia BETWEEN 1 AND 28),
    -- Ley 675 art. 30: hasta 1,5 veces el interés bancario corriente, o la tasa inferior
    -- aprobada por la asamblea (mensual, en %).
    factor_mora                NUMERIC(4,3)  NOT NULL CHECK (factor_mora > 0 AND factor_mora <= 1.5),
    tasa_mora_mensual          NUMERIC(6,4)  CHECK (tasa_mora_mensual > 0),
    imputacion                 TEXT          NOT NULL
                               CHECK (imputacion IN ('intereses_primero', 'por_antiguedad')),
    permite_elegir_obligacion  BOOLEAN       NOT NULL DEFAULT FALSE,
    -- Ley 675 art. 35: al menos 1 % del presupuesto, salvo suspensión aprobada.
    porcentaje_fondo           NUMERIC(5,2)  NOT NULL CHECK (porcentaje_fondo BETWEEN 0 AND 100),
    fondo_suspendido           BOOLEAN       NOT NULL DEFAULT FALSE,
    pregunta_id                UUID,
    creada_por                 UUID,
    creada_en                  TIMESTAMPTZ   NOT NULL,
    CHECK ((pronto_pago_porcentaje IS NULL) = (pronto_pago_dia IS NULL)),
    CHECK (fondo_suspendido OR porcentaje_fondo >= 1),
    UNIQUE (tenant_id, version),
    UNIQUE (tenant_id, vigente_desde),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Fase 2 · Presupuesto
-- ---------------------------------------------------------------------
CREATE TABLE presupuestos (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    anio           SMALLINT    NOT NULL CHECK (anio BETWEEN 2000 AND 2100),
    version        INTEGER     NOT NULL CHECK (version > 0),
    estado         TEXT        NOT NULL DEFAULT 'borrador'
                   CHECK (estado IN ('borrador', 'vigente', 'reemplazado')),
    descripcion    TEXT        CHECK (char_length(descripcion) <= 300),
    pregunta_id    UUID,
    vigente_desde  DATE,
    aprobado_por   UUID,
    creado_por     UUID,
    creado_en      TIMESTAMPTZ NOT NULL,
    CHECK ((estado = 'borrador') = (vigente_desde IS NULL)),
    UNIQUE (tenant_id, anio, version),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, pregunta_id) REFERENCES preguntas (tenant_id, id)
);
CREATE UNIQUE INDEX ux_presupuesto_vigente ON presupuestos (tenant_id, anio)
    WHERE estado = 'vigente';

CREATE TABLE rubros_presupuesto (
    id                  UUID            PRIMARY KEY,
    tenant_id           UUID            NOT NULL REFERENCES copropiedades (id),
    presupuesto_id      UUID            NOT NULL,
    -- Concepto de cobro que financia el rubro (ordinaria o de un módulo).
    concepto_cobro_id   UUID            NOT NULL,
    cuenta_contable_id  UUID            NOT NULL,
    centro_costo_id     UUID,
    nombre              TEXT            NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 150),
    -- Enero a diciembre.
    valores_mensuales   NUMERIC(18,2)[] NOT NULL CHECK (array_length(valores_mensuales, 1) = 12
                                                        AND 0 <= ALL (valores_mensuales)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, presupuesto_id)     REFERENCES presupuestos (tenant_id, id),
    FOREIGN KEY (tenant_id, concepto_cobro_id)  REFERENCES conceptos_cobro (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_contable_id) REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id)    REFERENCES centros_costo (tenant_id, id)
);
CREATE INDEX ix_rubros_presupuesto ON rubros_presupuesto (tenant_id, presupuesto_id);

-- Los rubros de un presupuesto aprobado son la decisión de la asamblea: inmutables.
CREATE FUNCTION exigir_presupuesto_borrador() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_estado TEXT;
BEGIN
    SELECT estado INTO v_estado FROM presupuestos
    WHERE id = coalesce(NEW.presupuesto_id, OLD.presupuesto_id)
    FOR SHARE;
    IF v_estado IS DISTINCT FROM 'borrador' THEN
        RAISE EXCEPTION 'el presupuesto no es un borrador (estado: %)',
            coalesce(v_estado, 'inexistente')
            USING ERRCODE = 'V0002';
    END IF;
    RETURN coalesce(NEW, OLD);
END
$$;
CREATE TRIGGER trg_rubros_solo_borrador
    BEFORE INSERT OR UPDATE OR DELETE ON rubros_presupuesto
    FOR EACH ROW EXECUTE FUNCTION exigir_presupuesto_borrador();

-- ---------------------------------------------------------------------
-- Fase 3 · Cuotas extraordinarias, liquidaciones, obligaciones y cuentas de cobro
-- ---------------------------------------------------------------------
CREATE TABLE cuotas_extraordinarias (
    id                      UUID          PRIMARY KEY,
    tenant_id               UUID          NOT NULL REFERENCES copropiedades (id),
    concepto_cobro_id       UUID          NOT NULL,
    finalidad               TEXT          NOT NULL CHECK (char_length(finalidad) BETWEEN 1 AND 300),
    valor_total             NUMERIC(18,2) NOT NULL CHECK (valor_total > 0),
    numero_cuotas           SMALLINT      NOT NULL CHECK (numero_cuotas BETWEEN 1 AND 120),
    primer_periodo          DATE          NOT NULL CHECK (extract(day FROM primer_periodo) = 1),
    -- Destinación específica (la regla contable decide el pasivo o el ingreso diferido, D6).
    destinacion_especifica  BOOLEAN       NOT NULL DEFAULT TRUE,
    pregunta_id             UUID,
    estado                  TEXT          NOT NULL DEFAULT 'vigente'
                            CHECK (estado IN ('vigente', 'anulada')),
    creada_por              UUID,
    creada_en               TIMESTAMPTZ   NOT NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, concepto_cobro_id) REFERENCES conceptos_cobro (tenant_id, id),
    FOREIGN KEY (tenant_id, pregunta_id)       REFERENCES preguntas (tenant_id, id)
);

CREATE TABLE liquidaciones_expensas (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES copropiedades (id),
    periodo           DATE          NOT NULL CHECK (extract(day FROM periodo) = 1),
    estado            TEXT          NOT NULL DEFAULT 'contabilizada'
                      CHECK (estado IN ('contabilizada', 'anulada')),
    presupuesto_id    UUID,
    politica_id       UUID          NOT NULL,
    total             NUMERIC(18,2) NOT NULL CHECK (total > 0),
    vencimiento       DATE          NOT NULL,
    comprobante_id    UUID          NOT NULL,
    motivo_anulacion  TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    liquidada_por     UUID,
    liquidada_en      TIMESTAMPTZ   NOT NULL,
    anulada_en        TIMESTAMPTZ,
    CHECK ((estado = 'anulada') = (motivo_anulacion IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, presupuesto_id) REFERENCES presupuestos (tenant_id, id),
    FOREIGN KEY (tenant_id, politica_id)    REFERENCES politicas_cartera (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id) REFERENCES comprobantes (tenant_id, id)
);
CREATE UNIQUE INDEX ux_liquidacion_periodo ON liquidaciones_expensas (tenant_id, periodo)
    WHERE estado = 'contabilizada';

-- Inmutable: un ajuste posterior sale en la siguiente cuenta de cobro.
CREATE TABLE cuentas_cobro (
    id                 UUID          PRIMARY KEY,
    tenant_id          UUID          NOT NULL REFERENCES copropiedades (id),
    anio               SMALLINT      NOT NULL,
    numero             INTEGER       NOT NULL CHECK (numero > 0),
    liquidacion_id     UUID          NOT NULL,
    bien_privado_id    UUID          NOT NULL,
    periodo            DATE          NOT NULL,
    deudor_persona_id  UUID          NOT NULL,
    saldo_anterior     NUMERIC(18,2) NOT NULL,
    cargos             NUMERIC(18,2) NOT NULL CHECK (cargos >= 0),
    total              NUMERIC(18,2) NOT NULL,
    vencimiento        DATE          NOT NULL,
    pronto_pago_hasta  DATE,
    descuento_pronto_pago NUMERIC(18,2) CHECK (descuento_pronto_pago > 0),
    referencia_pago    TEXT,
    creada_en          TIMESTAMPTZ   NOT NULL,
    CHECK ((pronto_pago_hasta IS NULL) = (descuento_pronto_pago IS NULL)),
    UNIQUE (tenant_id, anio, numero),
    UNIQUE (tenant_id, liquidacion_id, bien_privado_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, liquidacion_id)    REFERENCES liquidaciones_expensas (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)   REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, deudor_persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_cuentas_cobro_bien ON cuentas_cobro (tenant_id, bien_privado_id, periodo DESC);

CREATE TABLE obligaciones (
    id                       UUID          PRIMARY KEY,
    tenant_id                UUID          NOT NULL REFERENCES copropiedades (id),
    bien_privado_id          UUID          NOT NULL,
    concepto_cobro_id        UUID          NOT NULL,
    periodo                  DATE          NOT NULL CHECK (extract(day FROM periodo) = 1),
    valor                    NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    vencimiento              DATE          NOT NULL,
    -- El propietario representante del bien (decisión del plan 14): tercero en el mayor.
    deudor_persona_id        UUID          NOT NULL,
    origen                   TEXT          NOT NULL
                             CHECK (origen IN ('liquidacion', 'cargo', 'saldo_inicial', 'manual')),
    liquidacion_id           UUID,
    cuota_extraordinaria_id  UUID,
    cuenta_cobro_id          UUID,
    -- Solo las manuales tienen comprobante propio; las liquidadas van en el de la liquidación y
    -- el saldo inicial ya está en el comprobante de saldos iniciales.
    comprobante_id           UUID,
    descripcion              TEXT          CHECK (char_length(descripcion) <= 300),
    estado                   TEXT          NOT NULL DEFAULT 'vigente'
                             CHECK (estado IN ('vigente', 'anulada')),
    creada_en                TIMESTAMPTZ   NOT NULL,
    anulada_en               TIMESTAMPTZ,
    CHECK (origen NOT IN ('liquidacion', 'cargo') OR liquidacion_id IS NOT NULL),
    CHECK ((origen = 'manual') = (comprobante_id IS NOT NULL)),
    CHECK ((estado = 'anulada') = (anulada_en IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)         REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, concepto_cobro_id)       REFERENCES conceptos_cobro (tenant_id, id),
    FOREIGN KEY (tenant_id, deudor_persona_id)       REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, liquidacion_id)          REFERENCES liquidaciones_expensas (tenant_id, id),
    FOREIGN KEY (tenant_id, cuota_extraordinaria_id) REFERENCES cuotas_extraordinarias (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_cobro_id)         REFERENCES cuentas_cobro (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id)          REFERENCES comprobantes (tenant_id, id)
);
-- QA-005: reliquidar el mismo periodo no duplica cobros.
CREATE UNIQUE INDEX ux_obligacion_liquidada ON obligaciones (tenant_id, bien_privado_id,
    concepto_cobro_id, periodo,
    coalesce(cuota_extraordinaria_id, '00000000-0000-0000-0000-000000000000'::uuid))
    WHERE origen = 'liquidacion' AND estado = 'vigente';
CREATE INDEX ix_obligaciones_bien ON obligaciones (tenant_id, bien_privado_id, periodo, id);
CREATE INDEX ix_obligaciones_liquidacion ON obligaciones (tenant_id, liquidacion_id)
    WHERE liquidacion_id IS NOT NULL;

-- Reducción de una obligación con su comprobante: nota crédito o descuento por pronto pago.
CREATE TABLE ajustes_obligacion (
    id              UUID          PRIMARY KEY,
    tenant_id       UUID          NOT NULL REFERENCES copropiedades (id),
    obligacion_id   UUID          NOT NULL,
    tipo            TEXT          NOT NULL CHECK (tipo IN ('nota_credito', 'pronto_pago')),
    valor           NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    motivo          TEXT          NOT NULL CHECK (char_length(motivo) BETWEEN 5 AND 500),
    comprobante_id  UUID          NOT NULL,
    -- Recibo que ganó el descuento por pronto pago (anularlo revierte el descuento).
    recibo_id       UUID,
    creado_por      UUID,
    creado_en       TIMESTAMPTZ   NOT NULL,
    -- Anulado con el recibo que lo ganó o por su propio reverso (el comprobante se revierte).
    anulado_en      TIMESTAMPTZ,
    CHECK ((tipo = 'pronto_pago') = (recibo_id IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, obligacion_id)  REFERENCES obligaciones (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id) REFERENCES comprobantes (tenant_id, id),
    FOREIGN KEY (tenant_id, recibo_id)      REFERENCES recibos_caja (tenant_id, id)
);
CREATE INDEX ix_ajustes_obligacion ON ajustes_obligacion (tenant_id, obligacion_id);
CREATE INDEX ix_ajustes_recibo ON ajustes_obligacion (tenant_id, recibo_id)
    WHERE recibo_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Fase 4 · Recaudo: aplicaciones a obligaciones y cargos de las reservas
-- ---------------------------------------------------------------------
ALTER TABLE aplicaciones_pago
    ALTER COLUMN factura_venta_id DROP NOT NULL,
    ADD COLUMN obligacion_id UUID,
    ADD CONSTRAINT ck_aplicacion_destino CHECK (num_nonnulls(factura_venta_id, obligacion_id) = 1),
    ADD CONSTRAINT fk_aplicacion_obligacion
        FOREIGN KEY (tenant_id, obligacion_id) REFERENCES obligaciones (tenant_id, id);
CREATE INDEX ix_aplicaciones_obligacion ON aplicaciones_pago (tenant_id, obligacion_id)
    WHERE obligacion_id IS NOT NULL;

ALTER TABLE cargos_pendientes
    ADD COLUMN obligacion_id UUID,
    ADD CONSTRAINT ck_cargo_obligacion CHECK ((estado = 'aplicado') = (obligacion_id IS NOT NULL)),
    ADD CONSTRAINT fk_cargo_obligacion
        FOREIGN KEY (tenant_id, obligacion_id) REFERENCES obligaciones (tenant_id, id);
-- La liquidación marca el cargo aplicado (y su anulación lo devuelve a pendiente).
GRANT UPDATE (estado, obligacion_id) ON cargos_pendientes TO dad_app;

-- ---------------------------------------------------------------------
-- Solo inserción
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['politicas_cartera', 'cuentas_cobro'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t || '_solo_insercion', t);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t || '_sin_truncate', t);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------
-- RLS y privilegios
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'modulos_contribucion', 'conceptos_cobro', 'tarifas_concepto_cobro', 'politicas_cartera',
        'presupuestos', 'rubros_presupuesto', 'cuotas_extraordinarias', 'liquidaciones_expensas',
        'cuentas_cobro', 'obligaciones', 'ajustes_obligacion'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON modulos_contribucion, conceptos_cobro, presupuestos,
    cuotas_extraordinarias, liquidaciones_expensas, obligaciones, ajustes_obligacion TO dad_app;
GRANT SELECT, INSERT, DELETE ON tarifas_concepto_cobro, rubros_presupuesto TO dad_app;
GRANT SELECT, INSERT ON politicas_cartera, cuentas_cobro TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos y plantillas de rol
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('cartera.ver', 'copropiedad'),
    ('cartera.configurar', 'copropiedad'),
    ('cartera.liquidar', 'copropiedad'),
    ('cartera.ajustar', 'copropiedad'),
    ('presupuesto.gestionar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r,
     unnest(ARRAY['cartera.ver', 'cartera.configurar', 'cartera.liquidar', 'cartera.ajustar',
                  'presupuesto.gestionar']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'cartera.ver', NULL
FROM unnest(ARRAY['revisor_fiscal', 'consejero', 'presidente_consejo']) AS r;
