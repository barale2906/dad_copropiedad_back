-- Tesorería, cuentas por pagar, facturas de venta y conciliación bancaria (plan 13, ADR 0018).
--
-- Genérico: sirve a cualquier entidad y no conoce a la PH (el plan 14 agrega las obligaciones de
-- la copropiedad a `aplicaciones_pago`). Nada de aquí escribe asientos: cada documento se
-- contabiliza por el contabilizador del núcleo (`comprobantes.origen_tipo`/`origen_id`), que
-- garantiza un solo comprobante por documento.
--
-- Garantías que da la base de datos:
--   1. Una factura de proveedor no se registra dos veces (proveedor + prefijo + número) salvo
--      excepción con motivo y quien la autorizó.
--   2. Quien registra un egreso no lo autoriza (segregación de funciones).
--   3. Un extracto no se importa dos veces en la misma cuenta (SHA-256 del archivo).
--   4. Las conciliaciones son append-only: deshacer es un registro nuevo que apunta al original.
--   5. Un evento de la pasarela se registra una sola vez (proveedor + id del evento).

-- ---------------------------------------------------------------------
-- Catálogos de plataforma (plan 15 fase 1, adelantado): UVT y conceptos de retención.
-- Los mantiene la plataforma con `plataforma.normativa.gestionar`; cada concepto es una versión
-- con vigencia, así que una causación usa la tarifa vigente en la fecha de la factura.
-- ---------------------------------------------------------------------
CREATE TABLE uvt (
    anio    SMALLINT      PRIMARY KEY CHECK (anio BETWEEN 2000 AND 2999),
    valor   NUMERIC(12,2) NOT NULL CHECK (valor > 0),
    fuente  TEXT          NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300)
);
INSERT INTO uvt (anio, valor, fuente) VALUES
    (2025, 49799, 'Resolución DIAN 000193 de 2024'),
    (2026, 52374, 'Resolución DIAN 000238 de 2025');

CREATE TABLE conceptos_retencion (
    id               UUID          PRIMARY KEY,
    codigo           TEXT          NOT NULL CHECK (codigo ~ '^[a-z][a-z0-9_]{1,59}$'),
    impuesto         TEXT          NOT NULL CHECK (impuesto IN ('renta', 'iva', 'ica')),
    nombre           TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 3 AND 200),
    -- Base mínima en UVT del año de la operación (0 = desde el primer peso).
    base_minima_uvt  NUMERIC(10,2) NOT NULL CHECK (base_minima_uvt >= 0),
    -- Porcentaje (ReteICA por mil se expresa como porcentaje: 9,66 ‰ = 0,966).
    tarifa           NUMERIC(7,4)  NOT NULL CHECK (tarifa > 0 AND tarifa <= 100),
    -- Código PUC sugerido para la cuenta de la retención (la entidad usa la suya si existe).
    cuenta_sugerida  TEXT          CHECK (cuenta_sugerida ~ '^[1-9][0-9]{3,11}$'),
    municipio_dane   TEXT          CHECK (municipio_dane ~ '^[0-9]{5}$'),
    fuente           TEXT          NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    vigente_desde    DATE          NOT NULL,
    vigente_hasta    DATE          CHECK (vigente_hasta >= vigente_desde),
    CHECK ((impuesto = 'ica') OR municipio_dane IS NULL),
    UNIQUE (codigo, vigente_desde)
);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['uvt', 'conceptos_retencion'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY ver ON %I FOR SELECT USING (TRUE)', t);
        EXECUTE format('CREATE POLICY crear ON %I FOR INSERT WITH CHECK (app_plataforma())', t);
        EXECUTE format('CREATE POLICY editar ON %I FOR UPDATE USING (app_plataforma()) '
                       'WITH CHECK (app_plataforma())', t);
    END LOOP;
END $$;
GRANT SELECT, INSERT, UPDATE ON uvt, conceptos_retencion TO dad_app;

-- ---------------------------------------------------------------------
-- Fase 1 · Cuentas de tesorería, recibos y egresos
-- ---------------------------------------------------------------------
CREATE TABLE cuentas_tesoreria (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES entidades (id),
    tipo                TEXT        NOT NULL
                        CHECK (tipo IN ('banco', 'caja_general', 'caja_menor', 'fondo')),
    nombre              TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    banco               TEXT        CHECK (char_length(banco) BETWEEN 2 AND 120),
    -- Solo los últimos dígitos (`****1234`): el número completo no se guarda.
    numero_enmascarado  TEXT        CHECK (numero_enmascarado ~ '^\*{4}[0-9]{4}$'),
    tipo_cuenta         TEXT        CHECK (tipo_cuenta IN ('ahorros', 'corriente')),
    cuenta_contable_id  UUID        NOT NULL,
    destinacion         TEXT        CHECK (char_length(destinacion) <= 200),
    activa              BOOLEAN     NOT NULL DEFAULT TRUE,
    creada_en           TIMESTAMPTZ NOT NULL,
    CHECK ((tipo = 'banco') = (banco IS NOT NULL)),
    CHECK (tipo = 'banco' OR (numero_enmascarado IS NULL AND tipo_cuenta IS NULL)),
    UNIQUE (tenant_id, nombre),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_contable_id) REFERENCES cuentas_contables (tenant_id, id)
);
-- Una cuenta contable respalda una sola cuenta de tesorería (la conciliación la usa).
CREATE UNIQUE INDEX ux_tesoreria_cuenta_contable ON cuentas_tesoreria (tenant_id, cuenta_contable_id);

CREATE TABLE recibos_caja (
    id                   UUID          PRIMARY KEY,
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    cuenta_tesoreria_id  UUID          NOT NULL,
    -- NULL = pagador no identificado (consignación sin identificar).
    persona_id           UUID,
    fecha                DATE          NOT NULL,
    valor                NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    medio                TEXT          NOT NULL CHECK (medio IN (
                             'efectivo', 'transferencia', 'consignacion', 'pasarela', 'cheque')),
    referencia           TEXT          CHECK (char_length(referencia) <= 80),
    descripcion          TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    soporte_archivo_id   UUID,
    estado               TEXT          NOT NULL DEFAULT 'registrado'
                         CHECK (estado IN ('registrado', 'anulado')),
    comprobante_id       UUID          NOT NULL,
    motivo_anulacion     TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    registrado_por       UUID,
    registrado_en        TIMESTAMPTZ   NOT NULL,
    CHECK ((estado = 'anulado') = (motivo_anulacion IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)          REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id)  REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id)      REFERENCES comprobantes (tenant_id, id)
);
CREATE INDEX ix_recibos_fecha ON recibos_caja (tenant_id, fecha DESC, id);
CREATE INDEX ix_recibos_persona ON recibos_caja (tenant_id, persona_id) WHERE persona_id IS NOT NULL;

CREATE TABLE egresos (
    id                   UUID          PRIMARY KEY,
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    cuenta_tesoreria_id  UUID          NOT NULL,
    persona_id           UUID          NOT NULL,
    fecha                DATE          NOT NULL,
    valor                NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    medio                TEXT          NOT NULL CHECK (medio IN (
                             'efectivo', 'transferencia', 'consignacion', 'pse', 'cheque')),
    -- Número de cheque, de la transferencia o del PSE.
    referencia           TEXT          CHECK (char_length(referencia) <= 80),
    descripcion          TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    soporte_archivo_id   UUID,
    estado               TEXT          NOT NULL DEFAULT 'por_autorizar'
                         CHECK (estado IN ('por_autorizar', 'autorizado', 'pagado', 'anulado')),
    registrado_por       UUID          NOT NULL,
    registrado_en        TIMESTAMPTZ   NOT NULL,
    autorizado_por       UUID,
    autorizado_en        TIMESTAMPTZ,
    pagado_en            TIMESTAMPTZ,
    comprobante_id       UUID,
    motivo_anulacion     TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    -- Quien registra no autoriza.
    CHECK (autorizado_por IS NULL OR autorizado_por <> registrado_por),
    CHECK ((autorizado_por IS NULL) = (autorizado_en IS NULL)),
    CHECK (estado IN ('por_autorizar', 'anulado') OR autorizado_por IS NOT NULL),
    CHECK ((comprobante_id IS NOT NULL) = (pagado_en IS NOT NULL)),
    CHECK (estado <> 'pagado' OR comprobante_id IS NOT NULL),
    CHECK ((estado = 'anulado') = (motivo_anulacion IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)          REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id)  REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id)      REFERENCES comprobantes (tenant_id, id)
);
CREATE INDEX ix_egresos_estado ON egresos (tenant_id, estado, fecha DESC, id);

-- Pagos sin factura (caja menor, gastos bancarios, anticipos): la relación de gastos.
CREATE TABLE egreso_gastos (
    tenant_id        UUID          NOT NULL REFERENCES entidades (id),
    egreso_id        UUID          NOT NULL,
    orden            SMALLINT      NOT NULL CHECK (orden > 0),
    cuenta_id        UUID          NOT NULL,
    persona_id       UUID,
    centro_costo_id  UUID,
    valor            NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    descripcion      TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    PRIMARY KEY (tenant_id, egreso_id, orden),
    FOREIGN KEY (tenant_id, egreso_id)       REFERENCES egresos (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_id)       REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)      REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id) REFERENCES centros_costo (tenant_id, id)
);

-- ---------------------------------------------------------------------
-- Fase 2 · Facturas de proveedores y causación
-- ---------------------------------------------------------------------
CREATE TABLE facturas_proveedor (
    id                     UUID          PRIMARY KEY,
    tenant_id              UUID          NOT NULL REFERENCES entidades (id),
    persona_id             UUID          NOT NULL,
    prefijo                TEXT          NOT NULL DEFAULT '' CHECK (prefijo ~ '^[A-Z0-9]{0,10}$'),
    numero                 TEXT          NOT NULL CHECK (numero ~ '^[0-9A-Za-z-]{1,20}$'),
    fecha                  DATE          NOT NULL,
    vencimiento            DATE          NOT NULL CHECK (vencimiento >= fecha),
    descripcion            TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    -- Documento que la originó en otro módulo (p. ej. una orden de trabajo o un contrato).
    origen_tipo            TEXT          CHECK (origen_tipo ~ '^[a-z][a-z0-9_.]{2,79}$'),
    origen_id              UUID,
    cufe                   TEXT          CHECK (cufe ~ '^[0-9a-f]{96}$'),
    archivo_id             UUID,
    subtotal               NUMERIC(18,2) NOT NULL CHECK (subtotal > 0),
    iva                    NUMERIC(18,2) NOT NULL CHECK (iva >= 0),
    retenciones            NUMERIC(18,2) NOT NULL CHECK (retenciones >= 0),
    -- Lo que se le debe al proveedor: subtotal + IVA − retenciones.
    neto_pagar             NUMERIC(18,2) NOT NULL CHECK (neto_pagar > 0),
    pagado                 NUMERIC(18,2) NOT NULL DEFAULT 0
                           CHECK (pagado >= 0 AND pagado <= neto_pagar),
    estado                 TEXT          NOT NULL DEFAULT 'registrada' CHECK (estado IN (
                               'registrada', 'causada', 'pagada_parcial', 'pagada', 'anulada')),
    excepcion_duplicado    TEXT          CHECK (char_length(excepcion_duplicado) BETWEEN 5 AND 500),
    excepcion_autorizada_por UUID,
    comprobante_id         UUID,
    motivo_anulacion       TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    registrada_por         UUID,
    registrada_en          TIMESTAMPTZ   NOT NULL,
    CHECK ((origen_tipo IS NULL) = (origen_id IS NULL)),
    CHECK ((excepcion_duplicado IS NULL) = (excepcion_autorizada_por IS NULL)),
    CHECK (neto_pagar = subtotal + iva - retenciones),
    CHECK (estado IN ('registrada', 'anulada') OR comprobante_id IS NOT NULL),
    CHECK ((estado = 'anulada') = (motivo_anulacion IS NOT NULL)),
    CHECK (estado <> 'pagada' OR pagado = neto_pagar),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)     REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id)     REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id) REFERENCES comprobantes (tenant_id, id)
);
-- Garantía 1.
CREATE UNIQUE INDEX ux_factura_proveedor ON facturas_proveedor (tenant_id, persona_id, prefijo, numero)
    WHERE excepcion_duplicado IS NULL AND estado <> 'anulada';
CREATE INDEX ix_facturas_proveedor_estado ON facturas_proveedor (tenant_id, estado, vencimiento, id);

CREATE TABLE lineas_factura_proveedor (
    tenant_id        UUID          NOT NULL REFERENCES entidades (id),
    factura_id       UUID          NOT NULL,
    orden            SMALLINT      NOT NULL CHECK (orden > 0),
    descripcion      TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    cuenta_id        UUID          NOT NULL,
    centro_costo_id  UUID,
    base             NUMERIC(18,2) NOT NULL CHECK (base > 0),
    iva              NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (iva >= 0),
    -- FALSE = el IVA es mayor valor del gasto (no descontable).
    iva_descontable  BOOLEAN       NOT NULL DEFAULT TRUE,
    PRIMARY KEY (tenant_id, factura_id, orden),
    FOREIGN KEY (tenant_id, factura_id)      REFERENCES facturas_proveedor (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_id)       REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id) REFERENCES centros_costo (tenant_id, id)
);

-- Retenciones calculadas por línea con la versión del concepto usada.
CREATE TABLE retenciones_factura_proveedor (
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    factura_id           UUID          NOT NULL,
    linea_orden          SMALLINT      NOT NULL,
    concepto_retencion_id UUID         NOT NULL REFERENCES conceptos_retencion (id),
    impuesto             TEXT          NOT NULL CHECK (impuesto IN ('renta', 'iva', 'ica')),
    base                 NUMERIC(18,2) NOT NULL CHECK (base > 0),
    tarifa               NUMERIC(7,4)  NOT NULL CHECK (tarifa > 0),
    valor                NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    PRIMARY KEY (tenant_id, factura_id, linea_orden, impuesto),
    FOREIGN KEY (tenant_id, factura_id, linea_orden)
        REFERENCES lineas_factura_proveedor (tenant_id, factura_id, orden)
);

CREATE TABLE egreso_facturas (
    tenant_id             UUID          NOT NULL REFERENCES entidades (id),
    egreso_id             UUID          NOT NULL,
    factura_proveedor_id  UUID          NOT NULL,
    valor                 NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    PRIMARY KEY (tenant_id, egreso_id, factura_proveedor_id),
    FOREIGN KEY (tenant_id, egreso_id)            REFERENCES egresos (tenant_id, id),
    FOREIGN KEY (tenant_id, factura_proveedor_id) REFERENCES facturas_proveedor (tenant_id, id)
);
CREATE INDEX ix_egreso_facturas_factura ON egreso_facturas (tenant_id, factura_proveedor_id);

-- ---------------------------------------------------------------------
-- Fase 2b · Facturas de venta y cuentas por cobrar genéricas
-- ---------------------------------------------------------------------
CREATE TABLE facturas_venta (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES entidades (id),
    persona_id        UUID          NOT NULL,
    -- Número del comprobante `FV` al emitir (la numeración DIAN llega con el plan 15).
    numero            INTEGER       CHECK (numero > 0),
    fecha             DATE          NOT NULL,
    vencimiento       DATE          NOT NULL CHECK (vencimiento >= fecha),
    descripcion       TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    -- Documento que la originó (p. ej. una reserva de la PH): el núcleo no lo interpreta.
    origen_tipo       TEXT          CHECK (origen_tipo ~ '^[a-z][a-z0-9_.]{2,79}$'),
    origen_id         UUID,
    subtotal          NUMERIC(18,2) NOT NULL CHECK (subtotal > 0),
    iva               NUMERIC(18,2) NOT NULL CHECK (iva >= 0),
    total             NUMERIC(18,2) NOT NULL CHECK (total = subtotal + iva),
    pagado            NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (pagado >= 0 AND pagado <= total),
    estado            TEXT          NOT NULL DEFAULT 'borrador' CHECK (estado IN (
                          'borrador', 'emitida', 'pagada_parcial', 'pagada', 'anulada')),
    comprobante_id    UUID,
    motivo_anulacion  TEXT          CHECK (char_length(motivo_anulacion) BETWEEN 5 AND 500),
    creada_por        UUID,
    creada_en         TIMESTAMPTZ   NOT NULL,
    emitida_en        TIMESTAMPTZ,
    CHECK ((origen_tipo IS NULL) = (origen_id IS NULL)),
    CHECK (estado <> 'borrador' OR (comprobante_id IS NULL AND numero IS NULL AND emitida_en IS NULL)),
    CHECK (estado IN ('borrador', 'anulada')
           OR (comprobante_id IS NOT NULL AND numero IS NOT NULL AND emitida_en IS NOT NULL)),
    CHECK ((estado = 'anulada') = (motivo_anulacion IS NOT NULL)),
    CHECK (estado <> 'pagada' OR pagado = total),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)     REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id) REFERENCES comprobantes (tenant_id, id)
);
CREATE INDEX ix_facturas_venta_estado ON facturas_venta (tenant_id, estado, vencimiento, id);
CREATE INDEX ix_facturas_venta_persona ON facturas_venta (tenant_id, persona_id);
CREATE UNIQUE INDEX ux_factura_venta_origen ON facturas_venta (tenant_id, origen_tipo, origen_id)
    WHERE origen_id IS NOT NULL AND estado <> 'anulada';

CREATE TABLE lineas_factura_venta (
    tenant_id        UUID          NOT NULL REFERENCES entidades (id),
    factura_id       UUID          NOT NULL,
    orden            SMALLINT      NOT NULL CHECK (orden > 0),
    descripcion      TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    cuenta_id        UUID          NOT NULL,
    centro_costo_id  UUID,
    cantidad         NUMERIC(12,4) NOT NULL CHECK (cantidad > 0),
    valor_unitario   NUMERIC(18,2) NOT NULL CHECK (valor_unitario > 0),
    -- Porcentaje que eligió quien factura (nunca un 19 % automático).
    tarifa_iva       NUMERIC(5,2)  NOT NULL DEFAULT 0 CHECK (tarifa_iva >= 0 AND tarifa_iva <= 100),
    subtotal         NUMERIC(18,2) NOT NULL CHECK (subtotal > 0),
    iva              NUMERIC(18,2) NOT NULL CHECK (iva >= 0),
    PRIMARY KEY (tenant_id, factura_id, orden),
    FOREIGN KEY (tenant_id, factura_id)      REFERENCES facturas_venta (tenant_id, id)
                                             ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, cuenta_id)       REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id) REFERENCES centros_costo (tenant_id, id)
);

-- Un solo mecanismo de recaudo: el recibo se aplica a facturas de venta (y, con el plan 14, a
-- las obligaciones de la PH). Una aplicación posterior al recibo lleva su propio comprobante.
CREATE TABLE aplicaciones_pago (
    id                UUID          PRIMARY KEY,
    tenant_id         UUID          NOT NULL REFERENCES entidades (id),
    recibo_id         UUID          NOT NULL,
    factura_venta_id  UUID          NOT NULL,
    valor             NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    aplicada_en       TIMESTAMPTZ   NOT NULL,
    -- Solo si se aplicó después del recibo (reclasificación del anticipo).
    comprobante_id    UUID,
    anulada_en        TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, recibo_id)        REFERENCES recibos_caja (tenant_id, id),
    FOREIGN KEY (tenant_id, factura_venta_id) REFERENCES facturas_venta (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id)   REFERENCES comprobantes (tenant_id, id)
);
CREATE INDEX ix_aplicaciones_recibo ON aplicaciones_pago (tenant_id, recibo_id);
CREATE INDEX ix_aplicaciones_factura ON aplicaciones_pago (tenant_id, factura_venta_id);

-- ---------------------------------------------------------------------
-- Fase 3 · Extractos y conciliación bancaria
-- ---------------------------------------------------------------------
CREATE TABLE extractos_bancarios (
    id                   UUID          PRIMARY KEY,
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    cuenta_tesoreria_id  UUID          NOT NULL,
    desde                DATE          NOT NULL,
    hasta                DATE          NOT NULL CHECK (hasta >= desde),
    formato              TEXT          NOT NULL CHECK (formato IN ('planilla', 'asobancaria_2001')),
    version_lector       TEXT          NOT NULL CHECK (version_lector ~ '^[a-z0-9_]+-[0-9]+$'),
    archivo_id           UUID          NOT NULL,
    sha256               TEXT          NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    saldo_inicial        NUMERIC(18,2),
    saldo_final          NUMERIC(18,2),
    cargado_por          UUID,
    cargado_en           TIMESTAMPTZ   NOT NULL,
    -- Garantía 3.
    UNIQUE (tenant_id, cuenta_tesoreria_id, sha256),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id),
    FOREIGN KEY (tenant_id, archivo_id)          REFERENCES archivos (tenant_id, id)
);

CREATE TABLE movimientos_extracto (
    id                   UUID          PRIMARY KEY,
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    extracto_id          UUID          NOT NULL,
    cuenta_tesoreria_id  UUID          NOT NULL,
    orden                INTEGER       NOT NULL CHECK (orden > 0),
    fecha                DATE          NOT NULL,
    descripcion          TEXT          NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 300),
    referencia           TEXT          CHECK (char_length(referencia) <= 80),
    -- Positivo = entra dinero (crédito del banco); negativo = sale.
    valor                NUMERIC(18,2) NOT NULL CHECK (valor <> 0),
    estado               TEXT          NOT NULL DEFAULT 'pendiente'
                         CHECK (estado IN ('pendiente', 'conciliado', 'partida_conciliatoria')),
    UNIQUE (tenant_id, extracto_id, orden),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, extracto_id)         REFERENCES extractos_bancarios (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id)
);
CREATE INDEX ix_movimientos_pendientes ON movimientos_extracto (tenant_id, cuenta_tesoreria_id, estado, fecha);

-- Garantía 4: una conciliación agrupa movimientos del extracto y líneas contables de la cuenta
-- bancaria cuyas sumas coinciden; deshacer es otra fila que apunta a la original.
CREATE TABLE conciliaciones (
    id                   UUID        PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES entidades (id),
    cuenta_tesoreria_id  UUID        NOT NULL,
    accion               TEXT        NOT NULL CHECK (accion IN ('conciliar', 'deshacer')),
    modo                 TEXT        NOT NULL CHECK (modo IN ('automatica', 'manual')),
    deshace_a            UUID,
    por                  UUID,
    en                   TIMESTAMPTZ NOT NULL,
    CHECK ((accion = 'deshacer') = (deshace_a IS NOT NULL)),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id),
    FOREIGN KEY (tenant_id, deshace_a)           REFERENCES conciliaciones (tenant_id, id)
);
CREATE UNIQUE INDEX ux_conciliacion_deshecha ON conciliaciones (tenant_id, deshace_a)
    WHERE deshace_a IS NOT NULL;

CREATE TABLE conciliacion_partidas (
    tenant_id        UUID     NOT NULL REFERENCES entidades (id),
    conciliacion_id  UUID     NOT NULL,
    movimiento_id    UUID,
    comprobante_id   UUID,
    linea_orden      SMALLINT,
    CHECK ((movimiento_id IS NULL) <> (comprobante_id IS NULL)),
    CHECK ((comprobante_id IS NULL) = (linea_orden IS NULL)),
    FOREIGN KEY (tenant_id, conciliacion_id) REFERENCES conciliaciones (tenant_id, id),
    FOREIGN KEY (tenant_id, movimiento_id)   REFERENCES movimientos_extracto (tenant_id, id),
    FOREIGN KEY (tenant_id, comprobante_id, linea_orden)
        REFERENCES lineas_comprobante (tenant_id, comprobante_id, orden)
);
CREATE INDEX ix_partidas_conciliacion ON conciliacion_partidas (tenant_id, conciliacion_id);
CREATE INDEX ix_partidas_movimiento ON conciliacion_partidas (tenant_id, movimiento_id)
    WHERE movimiento_id IS NOT NULL;
CREATE INDEX ix_partidas_linea ON conciliacion_partidas (tenant_id, comprobante_id, linea_orden)
    WHERE comprobante_id IS NOT NULL;

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['conciliaciones', 'conciliacion_partidas'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t || '_solo_insercion', t);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t || '_sin_truncate', t);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------
-- Fase 4 · Eventos de la pasarela de pagos (el adaptador llega al elegir la pasarela)
-- ---------------------------------------------------------------------
CREATE TABLE eventos_pasarela (
    id                   UUID          PRIMARY KEY,
    tenant_id            UUID          NOT NULL REFERENCES entidades (id),
    proveedor            TEXT          NOT NULL CHECK (proveedor ~ '^[a-z][a-z0-9_]{1,39}$'),
    evento_externo       TEXT          NOT NULL CHECK (char_length(evento_externo) BETWEEN 1 AND 120),
    referencia           TEXT          NOT NULL CHECK (char_length(referencia) BETWEEN 1 AND 120),
    estado               TEXT          NOT NULL CHECK (estado IN (
                             'pendiente', 'aprobado', 'liquidado', 'reversado', 'contracargo')),
    valor                NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    comision             NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (comision >= 0),
    retenciones          NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (retenciones >= 0),
    cuenta_tesoreria_id  UUID          NOT NULL,
    persona_id           UUID,
    recibo_id            UUID,
    ocurrido_en          TIMESTAMPTZ   NOT NULL,
    recibido_en          TIMESTAMPTZ   NOT NULL,
    -- Garantía 5.
    UNIQUE (proveedor, evento_externo),
    FOREIGN KEY (tenant_id, cuenta_tesoreria_id) REFERENCES cuentas_tesoreria (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)          REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, recibo_id)           REFERENCES recibos_caja (tenant_id, id)
);
CREATE INDEX ix_eventos_pasarela_referencia ON eventos_pasarela (tenant_id, referencia, ocurrido_en);
CREATE TRIGGER eventos_pasarela_solo_insercion BEFORE UPDATE OR DELETE ON eventos_pasarela
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

-- ---------------------------------------------------------------------
-- RLS y permisos de la aplicación
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'cuentas_tesoreria', 'recibos_caja', 'egresos', 'egreso_gastos', 'facturas_proveedor',
        'lineas_factura_proveedor', 'retenciones_factura_proveedor', 'egreso_facturas',
        'facturas_venta', 'lineas_factura_venta', 'aplicaciones_pago', 'extractos_bancarios',
        'movimientos_extracto', 'conciliaciones', 'conciliacion_partidas', 'eventos_pasarela'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON cuentas_tesoreria, recibos_caja, egresos, facturas_proveedor,
    facturas_venta, aplicaciones_pago, movimientos_extracto TO dad_app;
GRANT SELECT, INSERT ON egreso_gastos, lineas_factura_proveedor, retenciones_factura_proveedor,
    egreso_facturas, extractos_bancarios, conciliaciones, conciliacion_partidas,
    eventos_pasarela TO dad_app;
GRANT SELECT, INSERT, DELETE ON lineas_factura_venta TO dad_app;
GRANT DELETE ON facturas_venta TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos y plantillas de rol
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('tesoreria.configurar', 'copropiedad'),
    ('tesoreria.recibos.registrar', 'copropiedad'),
    ('tesoreria.egresos.registrar', 'copropiedad'),
    ('tesoreria.egresos.autorizar', 'copropiedad'),
    ('tesoreria.facturas_proveedor.gestionar', 'copropiedad'),
    ('tesoreria.facturas_venta.gestionar', 'copropiedad'),
    ('tesoreria.conciliar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('contador'), 'copropiedad', p, NULL
FROM unnest(ARRAY[
    'tesoreria.configurar', 'tesoreria.recibos.registrar', 'tesoreria.egresos.registrar',
    'tesoreria.facturas_proveedor.gestionar', 'tesoreria.facturas_venta.gestionar',
    'tesoreria.conciliar']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('administrador'), 'copropiedad', p.codigo, NULL
FROM permisos p WHERE p.codigo LIKE 'tesoreria.%';

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'tesoreria.egresos.autorizar', NULL);
