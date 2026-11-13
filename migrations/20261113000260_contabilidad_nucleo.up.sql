-- Núcleo contable genérico (plan 12 fases 2–7, ADR 0017).
--
-- Sirve a cualquier entidad (no conoce a la PH). Garantías que da la base de datos:
--   1. Partida doble: un comprobante contabilizado tiene al menos dos líneas y la suma de
--      débitos es exactamente igual a la de créditos (trigger diferido al confirmar).
--   2. Solo se contabiliza en un periodo `abierto`, en cuentas de movimiento activas y con el
--      tercero y el centro de costo que cada cuenta exige (trigger con FOR SHARE del periodo).
--   3. Un comprobante contabilizado y sus líneas no se modifican ni se borran: se revierten.
--   4. Un documento origen produce un solo comprobante (índice único por origen).
-- La normativa contable no está aquí: el PUC es un catálogo y lo demás son plantillas que elige
-- el contador (principio P1 del plan 11).

-- ---------------------------------------------------------------------
-- Módulo vendible
-- ---------------------------------------------------------------------
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'bienes_comunes', 'control_acceso', 'asambleas', 'gestion_administrativa',
    'publicidad'));

-- ---------------------------------------------------------------------
-- Catálogo PUC (plataforma). Se carga completo con la importación de la plataforma; aquí solo
-- van las clases y los grupos del Decreto 2650 de 1993.
-- ---------------------------------------------------------------------
CREATE TABLE catalogo_puc (
    codigo          TEXT        PRIMARY KEY
                    CHECK (codigo ~ '^[1-9][0-9]*$' AND char_length(codigo) IN (1, 2, 4, 6)),
    nombre          TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 200),
    naturaleza      TEXT        NOT NULL CHECK (naturaleza IN ('debito', 'credito')),
    actualizado_en  TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE catalogo_puc ENABLE ROW LEVEL SECURITY;
CREATE POLICY puc_ver ON catalogo_puc FOR SELECT USING (TRUE);
CREATE POLICY puc_crear ON catalogo_puc FOR INSERT WITH CHECK (app_plataforma());
CREATE POLICY puc_editar ON catalogo_puc FOR UPDATE USING (app_plataforma())
    WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE ON catalogo_puc TO dad_app;

INSERT INTO catalogo_puc (codigo, nombre, naturaleza) VALUES
    ('1', 'Activo', 'debito'),
    ('11', 'Disponible', 'debito'),
    ('12', 'Inversiones', 'debito'),
    ('13', 'Deudores', 'debito'),
    ('14', 'Inventarios', 'debito'),
    ('15', 'Propiedades, planta y equipo', 'debito'),
    ('16', 'Intangibles', 'debito'),
    ('17', 'Diferidos', 'debito'),
    ('18', 'Otros activos', 'debito'),
    ('19', 'Valorizaciones', 'debito'),
    ('2', 'Pasivo', 'credito'),
    ('21', 'Obligaciones financieras', 'credito'),
    ('22', 'Proveedores', 'credito'),
    ('23', 'Cuentas por pagar', 'credito'),
    ('24', 'Impuestos, gravámenes y tasas', 'credito'),
    ('25', 'Obligaciones laborales', 'credito'),
    ('26', 'Pasivos estimados y provisiones', 'credito'),
    ('27', 'Diferidos', 'credito'),
    ('28', 'Otros pasivos', 'credito'),
    ('29', 'Bonos y papeles comerciales', 'credito'),
    ('3', 'Patrimonio', 'credito'),
    ('31', 'Capital social', 'credito'),
    ('32', 'Superávit de capital', 'credito'),
    ('33', 'Reservas', 'credito'),
    ('34', 'Revalorización del patrimonio', 'credito'),
    ('35', 'Dividendos o participaciones decretados en acciones, cuotas o partes de interés social', 'credito'),
    ('36', 'Resultados del ejercicio', 'credito'),
    ('37', 'Resultados de ejercicios anteriores', 'credito'),
    ('38', 'Superávit por valorizaciones', 'credito'),
    ('4', 'Ingresos', 'credito'),
    ('41', 'Operacionales', 'credito'),
    ('42', 'No operacionales', 'credito'),
    ('47', 'Ajustes por inflación', 'credito'),
    ('5', 'Gastos', 'debito'),
    ('51', 'Operacionales de administración', 'debito'),
    ('52', 'Operacionales de ventas', 'debito'),
    ('53', 'No operacionales', 'debito'),
    ('54', 'Impuesto de renta y complementarios', 'debito'),
    ('59', 'Ganancias y pérdidas', 'debito'),
    ('6', 'Costos de ventas', 'debito'),
    ('61', 'Costo de ventas y de prestación de servicios', 'debito'),
    ('62', 'Compras', 'debito'),
    ('7', 'Costos de producción o de operación', 'debito'),
    ('71', 'Materia prima', 'debito'),
    ('72', 'Mano de obra directa', 'debito'),
    ('73', 'Costos indirectos', 'debito'),
    ('74', 'Contratos de servicios', 'debito'),
    ('8', 'Cuentas de orden deudoras', 'debito'),
    ('81', 'Derechos contingentes', 'debito'),
    ('82', 'Deudoras fiscales', 'debito'),
    ('83', 'Deudoras de control', 'debito'),
    ('84', 'Derechos contingentes por contra (Cr)', 'credito'),
    ('85', 'Deudoras fiscales por contra (Cr)', 'credito'),
    ('86', 'Deudoras de control por contra (Cr)', 'credito'),
    ('9', 'Cuentas de orden acreedoras', 'credito'),
    ('91', 'Responsabilidades contingentes', 'credito'),
    ('92', 'Acreedoras fiscales', 'credito'),
    ('93', 'Acreedoras de control', 'credito'),
    ('94', 'Responsabilidades contingentes por contra (Db)', 'debito'),
    ('95', 'Acreedoras fiscales por contra (Db)', 'debito'),
    ('96', 'Acreedoras de control por contra (Db)', 'debito');

-- ---------------------------------------------------------------------
-- Plantillas contables (plataforma): estándar publicadas por la plataforma o propias de una
-- firma contable. El contenido es un documento tipado en Rust (`PlantillaContable`): se copia
-- entero a la entidad al aplicarlo.
-- ---------------------------------------------------------------------
CREATE TABLE plantillas_contables (
    id               UUID        PRIMARY KEY,
    ambito           TEXT        NOT NULL CHECK (ambito IN ('plataforma', 'organizacion')),
    organizacion_id  UUID        REFERENCES organizaciones (id),
    nombre           TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    tipo_empresa     TEXT        NOT NULL CHECK (tipo_empresa ~ '^[a-z][a-z0-9_]{1,59}$'),
    version          INTEGER     NOT NULL CHECK (version > 0),
    estado           TEXT        NOT NULL DEFAULT 'borrador'
                     CHECK (estado IN ('borrador', 'publicada', 'retirada')),
    contenido        JSONB       NOT NULL,
    autor_cuenta_id  UUID        REFERENCES cuentas (id),
    creada_en        TIMESTAMPTZ NOT NULL,
    publicada_en     TIMESTAMPTZ,
    CHECK ((ambito = 'organizacion') = (organizacion_id IS NOT NULL)),
    CHECK ((estado = 'borrador') = (publicada_en IS NULL)),
    UNIQUE NULLS NOT DISTINCT (ambito, organizacion_id, nombre, version)
);
CREATE INDEX ix_plantillas_contables_org ON plantillas_contables (organizacion_id)
    WHERE organizacion_id IS NOT NULL;
ALTER TABLE plantillas_contables ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON plantillas_contables
    USING (app_plataforma()) WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE ON plantillas_contables TO dad_app;

-- Una publicada no cambia: editarla es una versión nueva (solo cambia su estado a retirada).
CREATE FUNCTION proteger_plantilla_publicada() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF OLD.estado <> 'borrador' AND (NEW.contenido IS DISTINCT FROM OLD.contenido
            OR NEW.nombre IS DISTINCT FROM OLD.nombre OR NEW.version IS DISTINCT FROM OLD.version
            OR NEW.tipo_empresa IS DISTINCT FROM OLD.tipo_empresa) THEN
        RAISE EXCEPTION 'una plantilla publicada no se edita: cree una versión nueva'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER plantilla_publicada_inmutable BEFORE UPDATE ON plantillas_contables
    FOR EACH ROW EXECUTE FUNCTION proteger_plantilla_publicada();

-- Lo que una entidad puede aplicar: las estándar publicadas y las publicadas de las firmas
-- contables donde la cuenta es dueña o gestora. Devuelve lo mínimo.
CREATE FUNCTION plantillas_contables_disponibles(p_cuenta UUID)
RETURNS TABLE (id UUID, ambito TEXT, organizacion TEXT, nombre TEXT, tipo_empresa TEXT,
               version INTEGER, publicada_en TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT p.id, p.ambito, o.razon_social, p.nombre, p.tipo_empresa, p.version, p.publicada_en
    FROM plantillas_contables p
    LEFT JOIN organizaciones o ON o.id = p.organizacion_id
    WHERE p.estado = 'publicada'
      AND (p.ambito = 'plataforma'
           OR EXISTS (SELECT 1 FROM organizaciones_miembros m
                      WHERE m.organizacion_id = p.organizacion_id AND m.cuenta_id = p_cuenta
                        AND m.rol IN ('dueno', 'gestor')))
    ORDER BY p.tipo_empresa, p.nombre, p.version DESC
$$;
REVOKE ALL ON FUNCTION plantillas_contables_disponibles(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION plantillas_contables_disponibles(UUID) TO dad_app;

CREATE FUNCTION plantilla_contable_para_aplicar(p_plantilla UUID, p_cuenta UUID)
RETURNS TABLE (nombre TEXT, version INTEGER, contenido JSONB)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT p.nombre, p.version, p.contenido FROM plantillas_contables p
    WHERE p.id = p_plantilla AND p.estado = 'publicada'
      AND (p.ambito = 'plataforma'
           OR EXISTS (SELECT 1 FROM organizaciones_miembros m
                      WHERE m.organizacion_id = p.organizacion_id AND m.cuenta_id = p_cuenta
                        AND m.rol IN ('dueno', 'gestor')))
$$;
REVOKE ALL ON FUNCTION plantilla_contable_para_aplicar(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION plantilla_contable_para_aplicar(UUID, UUID) TO dad_app;

-- ---------------------------------------------------------------------
-- Plan de cuentas, centros de costo y configuración (por entidad)
-- ---------------------------------------------------------------------
CREATE TABLE cuentas_contables (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES entidades (id),
    codigo              TEXT        NOT NULL CHECK (codigo ~ '^[1-9][0-9]{0,11}$'),
    nombre              TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 200),
    naturaleza          TEXT        NOT NULL CHECK (naturaleza IN ('debito', 'credito')),
    acepta_movimiento   BOOLEAN     NOT NULL,
    exige_tercero       BOOLEAN     NOT NULL DEFAULT FALSE,
    exige_centro_costo  BOOLEAN     NOT NULL DEFAULT FALSE,
    activa              BOOLEAN     NOT NULL DEFAULT TRUE,
    creada_en           TIMESTAMPTZ NOT NULL,
    actualizada_en      TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id)
);

CREATE TABLE centros_costo (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES entidades (id),
    codigo     TEXT        NOT NULL CHECK (codigo ~ '^[A-Za-z0-9._-]{1,20}$'),
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    activo     BOOLEAN     NOT NULL DEFAULT TRUE,
    creado_en  TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id)
);

-- Cada cambio es una versión nueva; la vigente es la de mayor versión.
CREATE TABLE configuracion_contable (
    id                    UUID        PRIMARY KEY,
    tenant_id             UUID        NOT NULL REFERENCES entidades (id),
    version               INTEGER     NOT NULL CHECK (version > 0),
    marco_tecnico         TEXT        NOT NULL
                          CHECK (marco_tecnico IN ('grupo_1', 'grupo_2', 'grupo_3', 'otro')),
    moneda_funcional      TEXT        NOT NULL DEFAULT 'COP' CHECK (moneda_funcional = 'COP'),
    mes_inicio_ejercicio  SMALLINT    NOT NULL DEFAULT 1 CHECK (mes_inicio_ejercicio = 1),
    cuenta_resultado_id   UUID,
    soporte_archivo_id    UUID,
    plantilla_origen      TEXT        CHECK (char_length(plantilla_origen) <= 160),
    vigente_desde         DATE        NOT NULL,
    creada_por            UUID,
    creada_en             TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, version),
    FOREIGN KEY (tenant_id, cuenta_resultado_id) REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id)  REFERENCES archivos (tenant_id, id)
);

-- El tercero contable es la persona del tenant (no hay tabla `terceros`): datos fiscales.
ALTER TABLE personas
    ADD COLUMN organizacion_id            UUID REFERENCES organizaciones (id),
    ADD COLUMN regimen_iva                TEXT CHECK (regimen_iva IN (
                                              'responsable', 'no_responsable', 'simple')),
    ADD COLUMN responsabilidades_fiscales TEXT[] NOT NULL DEFAULT '{}'
                                          CHECK (responsabilidades_fiscales <@ ARRAY[
                                              'O-13', 'O-15', 'O-23', 'O-47', 'R-99-PN']),
    ADD COLUMN actividad_ciiu             TEXT CHECK (actividad_ciiu ~ '^[0-9]{4}$'),
    ADD COLUMN direccion_fiscal           TEXT CHECK (char_length(direccion_fiscal) <= 200),
    ADD COLUMN municipio_fiscal_dane      TEXT CHECK (municipio_fiscal_dane ~ '^[0-9]{5}$'),
    ADD COLUMN correo_facturacion         TEXT CHECK (correo_facturacion = lower(correo_facturacion)
                                              AND correo_facturacion LIKE '%_@_%');
COMMENT ON COLUMN personas.direccion_fiscal IS 'dato_personal: dirección del RUT del tercero';
COMMENT ON COLUMN personas.correo_facturacion IS 'dato_personal: correo para documentos electrónicos';
CREATE INDEX ix_personas_organizacion ON personas (tenant_id, organizacion_id)
    WHERE organizacion_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Periodos (mes 13 = cierre anual) y reaperturas
-- ---------------------------------------------------------------------
CREATE TABLE periodos_contables (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES entidades (id),
    anio          SMALLINT    NOT NULL CHECK (anio BETWEEN 2000 AND 2999),
    mes           SMALLINT    NOT NULL CHECK (mes BETWEEN 1 AND 13),
    estado        TEXT        NOT NULL DEFAULT 'abierto'
                  CHECK (estado IN ('abierto', 'en_revision', 'cerrado')),
    cerrado_por   UUID,
    cerrado_en    TIMESTAMPTZ,
    UNIQUE (tenant_id, anio, mes),
    UNIQUE (tenant_id, id)
);

CREATE TABLE reaperturas_periodo (
    id              UUID        PRIMARY KEY,
    tenant_id       UUID        NOT NULL REFERENCES entidades (id),
    periodo_id      UUID        NOT NULL,
    motivo          TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 5 AND 1000),
    alcance         TEXT        CHECK (char_length(alcance) <= 1000),
    solicitada_por  UUID        NOT NULL,
    solicitada_en   TIMESTAMPTZ NOT NULL,
    estado          TEXT        NOT NULL DEFAULT 'pendiente'
                    CHECK (estado IN ('pendiente', 'aprobada', 'rechazada')),
    decidida_por    UUID,
    decidida_en     TIMESTAMPTZ,
    CHECK ((estado = 'pendiente') = (decidida_por IS NULL)),
    -- Quien solicita no aprueba (segregación de funciones).
    CHECK (decidida_por IS NULL OR decidida_por <> solicitada_por),
    FOREIGN KEY (tenant_id, periodo_id) REFERENCES periodos_contables (tenant_id, id)
);
CREATE UNIQUE INDEX ux_reapertura_pendiente ON reaperturas_periodo (tenant_id, periodo_id)
    WHERE estado = 'pendiente';

-- ---------------------------------------------------------------------
-- Tipos de comprobante y comprobantes
-- ---------------------------------------------------------------------
CREATE TABLE tipos_comprobante (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES entidades (id),
    codigo     TEXT        NOT NULL CHECK (codigo ~ '^[A-Z][A-Z0-9]{1,4}$'),
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    manual     BOOLEAN     NOT NULL DEFAULT TRUE,
    activo     BOOLEAN     NOT NULL DEFAULT TRUE,
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, id)
);

-- Consecutivos por tipo de comprobante y año en la tabla existente.
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo') OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

CREATE TABLE reglas_contabilizacion (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES entidades (id),
    evento         TEXT        NOT NULL CHECK (evento ~ '^[a-z][a-z0-9_.]{2,79}$'),
    version        INTEGER     NOT NULL CHECK (version > 0),
    tipo_comprobante_id UUID   NOT NULL,
    descripcion    TEXT        CHECK (char_length(descripcion) <= 300),
    vigente_desde  DATE        NOT NULL,
    creada_por     UUID,
    creada_en      TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, evento, version),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, tipo_comprobante_id) REFERENCES tipos_comprobante (tenant_id, id)
);

CREATE TABLE lineas_regla (
    tenant_id       UUID     NOT NULL REFERENCES entidades (id),
    regla_id        UUID     NOT NULL,
    orden           SMALLINT NOT NULL CHECK (orden > 0),
    concepto        TEXT     NOT NULL CHECK (concepto ~ '^[a-z][a-z0-9_]{1,59}$'),
    cara            TEXT     NOT NULL CHECK (cara IN ('debito', 'credito')),
    cuenta_id       UUID     NOT NULL,
    tercero         TEXT     NOT NULL CHECK (tercero IN ('documento', 'ninguno')),
    centro          TEXT     NOT NULL CHECK (centro IN ('documento', 'fijo', 'ninguno')),
    centro_costo_id UUID,
    CHECK ((centro = 'fijo') = (centro_costo_id IS NOT NULL)),
    PRIMARY KEY (tenant_id, regla_id, orden),
    FOREIGN KEY (tenant_id, regla_id)        REFERENCES reglas_contabilizacion (tenant_id, id),
    FOREIGN KEY (tenant_id, cuenta_id)       REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id) REFERENCES centros_costo (tenant_id, id)
);

CREATE TABLE comprobantes (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES entidades (id),
    tipo_comprobante_id UUID        NOT NULL,
    periodo_id          UUID        NOT NULL,
    numero              INTEGER     CHECK (numero > 0),
    fecha               DATE        NOT NULL,
    descripcion         TEXT        NOT NULL CHECK (char_length(descripcion) BETWEEN 1 AND 500),
    estado              TEXT        NOT NULL DEFAULT 'borrador'
                        CHECK (estado IN ('borrador', 'contabilizado')),
    origen_tipo         TEXT        CHECK (origen_tipo ~ '^[a-z][a-z0-9_.]{2,79}$'),
    origen_id           UUID,
    regla_id            UUID,
    revierte_a          UUID,
    motivo_reverso      TEXT        CHECK (char_length(motivo_reverso) BETWEEN 5 AND 500),
    creado_por          UUID,
    creado_en           TIMESTAMPTZ NOT NULL,
    contabilizado_por   UUID,
    contabilizado_en    TIMESTAMPTZ,
    CHECK ((origen_tipo IS NULL) = (origen_id IS NULL)),
    CHECK ((revierte_a IS NULL) = (motivo_reverso IS NULL)),
    CHECK ((estado = 'contabilizado') = (numero IS NOT NULL AND contabilizado_en IS NOT NULL)),
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, tipo_comprobante_id, periodo_id, numero),
    FOREIGN KEY (tenant_id, tipo_comprobante_id) REFERENCES tipos_comprobante (tenant_id, id),
    FOREIGN KEY (tenant_id, periodo_id)          REFERENCES periodos_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, regla_id)            REFERENCES reglas_contabilizacion (tenant_id, id),
    FOREIGN KEY (tenant_id, revierte_a)          REFERENCES comprobantes (tenant_id, id)
);
CREATE UNIQUE INDEX ux_comprobante_origen ON comprobantes (tenant_id, origen_tipo, origen_id)
    WHERE origen_id IS NOT NULL;
CREATE UNIQUE INDEX ux_comprobante_reverso ON comprobantes (tenant_id, revierte_a)
    WHERE revierte_a IS NOT NULL;
CREATE INDEX ix_comprobantes_fecha ON comprobantes (tenant_id, fecha, id);
CREATE INDEX ix_comprobantes_periodo ON comprobantes (tenant_id, periodo_id);

CREATE TABLE lineas_comprobante (
    tenant_id        UUID          NOT NULL REFERENCES entidades (id),
    comprobante_id   UUID          NOT NULL,
    orden            SMALLINT      NOT NULL CHECK (orden > 0),
    cuenta_id        UUID          NOT NULL,
    persona_id       UUID,
    centro_costo_id  UUID,
    debito           NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (debito >= 0),
    credito          NUMERIC(18,2) NOT NULL DEFAULT 0 CHECK (credito >= 0),
    base             NUMERIC(18,2) CHECK (base >= 0),
    descripcion      TEXT          CHECK (char_length(descripcion) <= 300),
    -- Una sola cara por línea y nunca en cero.
    CHECK ((debito = 0) <> (credito = 0)),
    PRIMARY KEY (tenant_id, comprobante_id, orden),
    FOREIGN KEY (tenant_id, comprobante_id)  REFERENCES comprobantes (tenant_id, id)
                                             ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, cuenta_id)       REFERENCES cuentas_contables (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)      REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, centro_costo_id) REFERENCES centros_costo (tenant_id, id)
);
CREATE INDEX ix_lineas_cuenta ON lineas_comprobante (tenant_id, cuenta_id);
CREATE INDEX ix_lineas_persona ON lineas_comprobante (tenant_id, persona_id, cuenta_id)
    WHERE persona_id IS NOT NULL;

-- Garantías 2 y 3 al contabilizar y después.
CREATE FUNCTION proteger_comprobante() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE
    v_estado TEXT;
    v_anio   SMALLINT;
    v_mes    SMALLINT;
    v_mala   TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.estado = 'contabilizado' THEN
            RAISE EXCEPTION 'un comprobante contabilizado no se borra: se revierte'
                USING ERRCODE = 'insufficient_privilege';
        END IF;
        RETURN OLD;
    END IF;
    IF OLD.estado = 'contabilizado' THEN
        RAISE EXCEPTION 'un comprobante contabilizado no se modifica: se revierte'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF NEW.estado = 'contabilizado' THEN
        SELECT p.estado, p.anio, p.mes INTO v_estado, v_anio, v_mes
        FROM periodos_contables p WHERE p.tenant_id = NEW.tenant_id AND p.id = NEW.periodo_id
        FOR SHARE;
        IF v_estado IS DISTINCT FROM 'abierto' THEN
            RAISE EXCEPTION 'el periodo del comprobante no está abierto'
                USING ERRCODE = 'check_violation', CONSTRAINT = 'ck_periodo_abierto';
        END IF;
        IF extract(year FROM NEW.fecha) <> v_anio
           OR (v_mes <= 12 AND extract(month FROM NEW.fecha) <> v_mes)
           OR (v_mes = 13 AND (extract(month FROM NEW.fecha) <> 12 OR extract(day FROM NEW.fecha) <> 31)) THEN
            RAISE EXCEPTION 'la fecha del comprobante no corresponde a su periodo'
                USING ERRCODE = 'check_violation', CONSTRAINT = 'ck_fecha_periodo';
        END IF;
        SELECT c.codigo INTO v_mala
        FROM lineas_comprobante l
        JOIN cuentas_contables c ON c.tenant_id = l.tenant_id AND c.id = l.cuenta_id
        WHERE l.tenant_id = NEW.tenant_id AND l.comprobante_id = NEW.id
          AND (NOT c.acepta_movimiento OR NOT c.activa
               OR (c.exige_tercero AND l.persona_id IS NULL)
               OR (c.exige_centro_costo AND l.centro_costo_id IS NULL))
        LIMIT 1;
        IF v_mala IS NOT NULL THEN
            RAISE EXCEPTION 'la cuenta % no admite la línea (movimiento, tercero o centro de costo)', v_mala
                USING ERRCODE = 'check_violation', CONSTRAINT = 'ck_lineas_cuenta';
        END IF;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER comprobante_protegido BEFORE UPDATE OR DELETE ON comprobantes
    FOR EACH ROW EXECUTE FUNCTION proteger_comprobante();

CREATE FUNCTION proteger_lineas_comprobante() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE
    v_fila RECORD;
BEGIN
    v_fila := CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    IF EXISTS (SELECT 1 FROM comprobantes c WHERE c.tenant_id = v_fila.tenant_id
               AND c.id = v_fila.comprobante_id AND c.estado = 'contabilizado') THEN
        RAISE EXCEPTION 'las líneas de un comprobante contabilizado no cambian'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN v_fila;
END
$$;
CREATE TRIGGER lineas_protegidas BEFORE INSERT OR UPDATE OR DELETE ON lineas_comprobante
    FOR EACH ROW EXECUTE FUNCTION proteger_lineas_comprobante();

-- Garantía 1, al confirmar la transacción.
CREATE FUNCTION exigir_partida_doble() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE
    v_lineas  INTEGER;
    v_debito  NUMERIC;
    v_credito NUMERIC;
BEGIN
    SELECT count(*), coalesce(sum(debito), 0), coalesce(sum(credito), 0)
    INTO v_lineas, v_debito, v_credito
    FROM lineas_comprobante WHERE tenant_id = NEW.tenant_id AND comprobante_id = NEW.id;
    IF v_lineas < 2 OR v_debito <> v_credito THEN
        RAISE EXCEPTION 'partida doble: % líneas, débitos % y créditos %', v_lineas, v_debito, v_credito
            USING ERRCODE = 'check_violation', CONSTRAINT = 'ck_partida_doble';
    END IF;
    RETURN NULL;
END
$$;
CREATE CONSTRAINT TRIGGER comprobante_balanceado
    AFTER INSERT OR UPDATE ON comprobantes DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW WHEN (NEW.estado = 'contabilizado')
    EXECUTE FUNCTION exigir_partida_doble();

CREATE TRIGGER comprobantes_sin_truncate BEFORE TRUNCATE ON comprobantes
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER lineas_comprobante_sin_truncate BEFORE TRUNCATE ON lineas_comprobante
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

-- ---------------------------------------------------------------------
-- Estados financieros: rubros (vienen de la plantilla) y notas versionadas
-- ---------------------------------------------------------------------
CREATE TABLE rubros_estado_financiero (
    id          UUID     PRIMARY KEY,
    tenant_id   UUID     NOT NULL REFERENCES entidades (id),
    estado      TEXT     NOT NULL CHECK (estado IN ('situacion_financiera', 'resultados')),
    codigo      TEXT     NOT NULL CHECK (codigo ~ '^[a-z][a-z0-9_]{1,59}$'),
    nombre      TEXT     NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 160),
    orden       SMALLINT NOT NULL CHECK (orden > 0),
    -- Prefijos de código de cuenta que suma el rubro (p. ej. {'11','12'}).
    prefijos    TEXT[]   NOT NULL CHECK (cardinality(prefijos) > 0),
    UNIQUE (tenant_id, estado, codigo),
    UNIQUE (tenant_id, id)
);

CREATE TABLE notas_estados (
    id          UUID        PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES entidades (id),
    anio        SMALLINT    NOT NULL CHECK (anio BETWEEN 2000 AND 2999),
    numero      SMALLINT    NOT NULL CHECK (numero > 0),
    version     INTEGER     NOT NULL CHECK (version > 0),
    titulo      TEXT        NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 160),
    texto       TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 20000),
    rubro_id    UUID,
    creada_por  UUID,
    creada_en   TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, anio, numero, version),
    FOREIGN KEY (tenant_id, rubro_id) REFERENCES rubros_estado_financiero (tenant_id, id)
);
CREATE TRIGGER notas_estados_solo_insercion BEFORE UPDATE OR DELETE ON notas_estados
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

-- ---------------------------------------------------------------------
-- RLS y permisos de la aplicación
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'cuentas_contables', 'centros_costo', 'configuracion_contable', 'periodos_contables',
        'reaperturas_periodo', 'tipos_comprobante', 'reglas_contabilizacion', 'lineas_regla',
        'comprobantes', 'lineas_comprobante', 'rubros_estado_financiero', 'notas_estados'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON cuentas_contables, centros_costo, periodos_contables,
    reaperturas_periodo, tipos_comprobante TO dad_app;
GRANT SELECT, INSERT ON configuracion_contable, reglas_contabilizacion, lineas_regla,
    notas_estados TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON comprobantes, lineas_comprobante,
    rubros_estado_financiero TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos y plantillas de rol
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('contabilidad.configurar', 'copropiedad'),
    ('contabilidad.terceros.gestionar', 'copropiedad'),
    ('contabilidad.comprobantes.crear', 'copropiedad'),
    ('contabilidad.comprobantes.contabilizar', 'copropiedad'),
    ('contabilidad.comprobantes.revertir', 'copropiedad'),
    ('contabilidad.periodos.cerrar', 'copropiedad'),
    ('contabilidad.periodos.reabrir', 'copropiedad'),
    ('contabilidad.periodos.aprobar_reapertura', 'copropiedad'),
    ('contabilidad.exportar', 'copropiedad'),
    ('plataforma.plantillas_contables.publicar', 'plataforma'),
    ('plataforma.entidades.crear', 'plataforma'),
    ('plataforma.normativa.gestionar', 'plataforma');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('contador'), 'copropiedad', p, NULL
FROM unnest(ARRAY[
    'contabilidad.configurar', 'contabilidad.terceros.gestionar',
    'contabilidad.comprobantes.crear', 'contabilidad.comprobantes.contabilizar',
    'contabilidad.comprobantes.revertir', 'contabilidad.periodos.cerrar',
    'contabilidad.periodos.reabrir', 'contabilidad.exportar']) AS p;

-- El administrador tiene todos los permisos de la entidad (plantilla sin lista).
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('administrador'), 'copropiedad', p.codigo, NULL
FROM permisos p WHERE p.codigo LIKE 'contabilidad.%' AND p.codigo <> 'contabilidad.ver';

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'contabilidad.exportar', NULL);

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla('superusuario'), 'plataforma', p, NULL
FROM unnest(ARRAY['plataforma.plantillas_contables.publicar',
                  'plataforma.normativa.gestionar', 'plataforma.entidades.crear']) AS p;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('comercial'), 'plataforma', 'plataforma.entidades.crear', NULL);
