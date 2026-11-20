-- Nómina (plan 16 fase 1, ADR 0025): catálogo laboral de plataforma, administradoras de la
-- seguridad social, empleados y contratos laborales versionados. Se vende sola (módulo
-- `nomina`) y sirve a cualquier entidad.
--
-- Ningún valor laboral vive en el código: el catálogo es de plataforma, versionado y de solo
-- inserción (vale la versión de mayor `vigente_desde` hasta la fecha).
--
-- Garantías de la base de datos:
--   1. Un empleado no tiene dos contratos que se crucen en fechas (EXCLUDE con btree_gist).
--   2. Todo contrato nace con su primera versión, que empieza el día del inicio (trigger
--      diferido).
--   3. Las versiones de un contrato son de solo inserción y avanzan en el tiempo: una
--      modificación es una versión nueva desde una fecha posterior a la última.
--   4. Un contrato terminado no se modifica; uno vigente solo cambia su fecha final (prórroga)
--      o se termina, y no se termina antes de su última versión.
--   5. El contrato de aprendizaje y el apoyo de sostenimiento van juntos.
-- Los errores de regla laboral usan el código `V0010`.

-- ---------------------------------------------------------------------
-- Módulo vendible e importación de empleados
-- ---------------------------------------------------------------------
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'facturacion_electronica', 'bienes_comunes',
    'control_acceso', 'asambleas', 'gestion_administrativa', 'nomina'));

ALTER TABLE importaciones DROP CONSTRAINT importaciones_tipo_check;
ALTER TABLE importaciones ADD CONSTRAINT importaciones_tipo_check
    CHECK (tipo IN ('bienes_privados', 'usuarios', 'empleados'));

-- ---------------------------------------------------------------------
-- Catálogo laboral de plataforma (versionado, solo inserción)
-- ---------------------------------------------------------------------
CREATE TABLE parametros_laborales (
    id             UUID          PRIMARY KEY,
    parametro      TEXT          NOT NULL CHECK (parametro IN (
                       'salario_minimo', 'auxilio_transporte', 'tope_auxilio_transporte',
                       'jornada_maxima_semanal', 'inicio_jornada_nocturna',
                       'fin_jornada_nocturna', 'recargo_nocturno', 'hora_extra_diurna',
                       'hora_extra_nocturna', 'recargo_dominical_festivo', 'salud_empleador',
                       'salud_empleado', 'pension_empleador', 'pension_empleado',
                       'caja_compensacion', 'icbf', 'sena', 'arl_clase_1', 'arl_clase_2',
                       'arl_clase_3', 'arl_clase_4', 'arl_clase_5', 'salario_integral_minimo',
                       'factor_salario_integral', 'limite_pagos_no_salariales',
                       'dias_incapacidad_empleador', 'incapacidad_eps', 'dias_cesantias',
                       'intereses_cesantias', 'dias_prima', 'dias_vacaciones',
                       'apoyo_aprendiz_lectiva', 'apoyo_aprendiz_productiva')),
    -- La unidad la da el parámetro (pesos, porcentaje, salarios mínimos, horas o días).
    valor          NUMERIC(18,4) NOT NULL CHECK (valor >= 0),
    -- Norma o documento de donde sale el valor (decreto, ley, concepto del especialista).
    fuente         TEXT          NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    vigente_desde  DATE          NOT NULL,
    creado_por     UUID,
    creado_en      TIMESTAMPTZ   NOT NULL,
    UNIQUE (parametro, vigente_desde)
);

-- EPS, fondos de pensiones y de cesantías, cajas de compensación y ARL, con su código de la
-- PILA. Es un catálogo regulado que mantiene la plataforma (no son organizaciones de clientes).
CREATE TABLE administradoras_seguridad_social (
    id         UUID        PRIMARY KEY,
    tipo       TEXT        NOT NULL CHECK (tipo IN ('eps', 'afp', 'cesantias', 'ccf', 'arl')),
    -- Código de la administradora en la PILA.
    codigo     TEXT        NOT NULL CHECK (codigo ~ '^[A-Z0-9-]{2,12}$'),
    nit        TEXT        NOT NULL CHECK (nit ~ '^[0-9]{1,15}-[0-9]$'),
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 3 AND 200),
    activa     BOOLEAN     NOT NULL DEFAULT TRUE,
    creada_en  TIMESTAMPTZ NOT NULL,
    UNIQUE (tipo, codigo)
);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['parametros_laborales', 'administradoras_seguridad_social'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY ver ON %I FOR SELECT USING (TRUE)', t);
        EXECUTE format('CREATE POLICY crear ON %I FOR INSERT WITH CHECK (app_plataforma())', t);
    END LOOP;
END $$;
CREATE POLICY editar ON administradoras_seguridad_social FOR UPDATE
    USING (app_plataforma()) WITH CHECK (app_plataforma());

CREATE TRIGGER parametros_laborales_solo_insercion BEFORE UPDATE OR DELETE
    ON parametros_laborales FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER parametros_laborales_sin_truncate BEFORE TRUNCATE
    ON parametros_laborales FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

GRANT SELECT, INSERT ON parametros_laborales TO dad_app;
GRANT SELECT, INSERT, UPDATE ON administradoras_seguridad_social TO dad_app;

-- ---------------------------------------------------------------------
-- Empleados y contratos laborales (por entidad)
-- ---------------------------------------------------------------------
CREATE TABLE empleados (
    id          UUID        PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES entidades (id),
    -- El empleado es una persona de la entidad (la misma del censo, de los terceros…).
    persona_id  UUID        NOT NULL,
    -- Código interno del empleado en la entidad.
    codigo      TEXT        NOT NULL CHECK (codigo ~ '^[A-Za-z0-9._-]{1,30}$'),
    creado_por  UUID,
    creado_en   TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, codigo),
    UNIQUE (tenant_id, persona_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_empleados_persona ON empleados (tenant_id, persona_id);

CREATE TABLE contratos_laborales (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES entidades (id),
    empleado_id         UUID        NOT NULL,
    tipo                TEXT        NOT NULL CHECK (tipo IN (
                            'termino_fijo', 'termino_indefinido', 'obra_labor', 'aprendizaje')),
    fecha_inicio        DATE        NOT NULL,
    -- Fecha final pactada (término fijo y aprendizaje); la prórroga la mueve.
    fecha_fin           DATE        CHECK (fecha_fin >= fecha_inicio),
    -- Obra o labor contratada.
    descripcion_obra    TEXT        CHECK (char_length(descripcion_obra) BETWEEN 3 AND 300),
    fecha_terminacion   DATE        CHECK (fecha_terminacion >= fecha_inicio),
    motivo_terminacion  TEXT        CHECK (motivo_terminacion IN (
                            'renuncia', 'mutuo_acuerdo', 'vencimiento_termino', 'fin_obra',
                            'justa_causa', 'sin_justa_causa', 'periodo_prueba', 'fallecimiento',
                            'otro')),
    creado_por          UUID,
    creado_en           TIMESTAMPTZ NOT NULL,
    terminado_por       UUID,
    terminado_en        TIMESTAMPTZ,
    CONSTRAINT ck_contrato_fecha_fin
        CHECK ((tipo IN ('termino_fijo', 'aprendizaje')) = (fecha_fin IS NOT NULL)),
    CONSTRAINT ck_contrato_obra CHECK ((tipo = 'obra_labor') = (descripcion_obra IS NOT NULL)),
    CONSTRAINT ck_contrato_terminacion CHECK (
        (fecha_terminacion IS NULL) = (motivo_terminacion IS NULL)
        AND (fecha_terminacion IS NULL) = (terminado_en IS NULL)),
    CONSTRAINT ck_contrato_termina_a_tiempo
        CHECK (fecha_terminacion IS NULL OR fecha_fin IS NULL OR fecha_terminacion <= fecha_fin),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, empleado_id) REFERENCES empleados (tenant_id, id),
    CONSTRAINT ex_contrato_por_empleado EXCLUDE USING gist (
        tenant_id WITH =, empleado_id WITH =,
        daterange(fecha_inicio, COALESCE(fecha_terminacion, fecha_fin), '[]') WITH &&)
);
CREATE INDEX ix_contratos_empleado ON contratos_laborales (tenant_id, empleado_id, fecha_inicio);

CREATE TABLE versiones_contrato_laboral (
    id                 UUID          PRIMARY KEY,
    tenant_id          UUID          NOT NULL REFERENCES entidades (id),
    contrato_id        UUID          NOT NULL,
    vigente_desde      DATE          NOT NULL,
    cargo              TEXT          NOT NULL CHECK (char_length(cargo) BETWEEN 2 AND 120),
    -- Salario mensual (o apoyo de sostenimiento del aprendiz).
    salario            NUMERIC(18,2) NOT NULL CHECK (salario > 0),
    tipo_salario       TEXT          NOT NULL CHECK (tipo_salario IN (
                           'ordinario', 'integral', 'apoyo_sostenimiento')),
    etapa_aprendizaje  TEXT          CHECK (etapa_aprendizaje IN ('lectiva', 'productiva')),
    jornada            TEXT          NOT NULL CHECK (jornada IN ('completa', 'parcial')),
    horas_semanales    NUMERIC(5,2)  CHECK (horas_semanales > 0 AND horas_semanales <= 60),
    periodicidad       TEXT          NOT NULL CHECK (periodicidad IN ('mensual', 'quincenal')),
    eps_id             UUID          NOT NULL REFERENCES administradoras_seguridad_social (id),
    afp_id             UUID          REFERENCES administradoras_seguridad_social (id),
    cesantias_id       UUID          REFERENCES administradoras_seguridad_social (id),
    ccf_id             UUID          REFERENCES administradoras_seguridad_social (id),
    arl_id             UUID          REFERENCES administradoras_seguridad_social (id),
    clase_riesgo       SMALLINT      CHECK (clase_riesgo BETWEEN 1 AND 5),
    -- Pensionado: no cotiza a pensión.
    pensionado         BOOLEAN       NOT NULL DEFAULT FALSE,
    medio_pago         TEXT          NOT NULL CHECK (medio_pago IN (
                           'transferencia', 'cheque', 'efectivo')),
    banco              TEXT          CHECK (char_length(banco) BETWEEN 2 AND 120),
    tipo_cuenta        TEXT          CHECK (tipo_cuenta IN ('ahorros', 'corriente')),
    numero_cuenta      TEXT          CHECK (numero_cuenta ~ '^[0-9]{4,20}$'),
    creada_por         UUID,
    creada_en          TIMESTAMPTZ   NOT NULL,
    CONSTRAINT ck_version_aprendiz
        CHECK ((tipo_salario = 'apoyo_sostenimiento') = (etapa_aprendizaje IS NOT NULL)),
    CONSTRAINT ck_version_jornada CHECK ((jornada = 'parcial') = (horas_semanales IS NOT NULL)),
    CONSTRAINT ck_version_cuenta CHECK (
        (medio_pago = 'transferencia')
            = (banco IS NOT NULL AND tipo_cuenta IS NOT NULL AND numero_cuenta IS NOT NULL)
        AND (medio_pago = 'transferencia'
             OR (banco IS NULL AND tipo_cuenta IS NULL AND numero_cuenta IS NULL))),
    CONSTRAINT ck_version_arl CHECK ((arl_id IS NULL) = (clase_riesgo IS NULL)),
    UNIQUE (contrato_id, vigente_desde),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, contrato_id) REFERENCES contratos_laborales (tenant_id, id)
);
CREATE INDEX ix_versiones_contrato ON versiones_contrato_laboral (contrato_id, vigente_desde);

COMMENT ON COLUMN versiones_contrato_laboral.salario IS 'dato_personal: salario del empleado';
COMMENT ON COLUMN versiones_contrato_laboral.banco IS 'dato_personal: banco de la cuenta de nómina';
COMMENT ON COLUMN versiones_contrato_laboral.tipo_cuenta IS
    'dato_personal: tipo de la cuenta de nómina';
COMMENT ON COLUMN versiones_contrato_laboral.numero_cuenta IS
    'dato_personal: número de la cuenta de nómina';
COMMENT ON COLUMN versiones_contrato_laboral.eps_id IS 'dato_personal: EPS del empleado';
COMMENT ON COLUMN versiones_contrato_laboral.afp_id IS
    'dato_personal: fondo de pensiones del empleado';
COMMENT ON COLUMN versiones_contrato_laboral.cesantias_id IS
    'dato_personal: fondo de cesantías del empleado';
COMMENT ON COLUMN versiones_contrato_laboral.pensionado IS
    'dato_personal: el empleado es pensionado';

-- Un contrato terminado no cambia; uno vigente solo se prorroga (fecha final mayor) o se
-- termina (no antes de su última versión). Nunca se borra.
CREATE FUNCTION proteger_contrato_laboral() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    ultima DATE;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'un contrato laboral no se borra: se termina' USING ERRCODE = 'V0010';
    END IF;
    IF OLD.fecha_terminacion IS NOT NULL THEN
        RAISE EXCEPTION 'el contrato ya terminó' USING ERRCODE = 'V0010';
    END IF;
    IF NEW.tipo <> OLD.tipo OR NEW.fecha_inicio <> OLD.fecha_inicio
       OR NEW.empleado_id <> OLD.empleado_id
       OR NEW.descripcion_obra IS DISTINCT FROM OLD.descripcion_obra THEN
        RAISE EXCEPTION 'el tipo, el inicio y el empleado de un contrato no cambian'
            USING ERRCODE = 'V0010';
    END IF;
    IF NEW.fecha_fin IS DISTINCT FROM OLD.fecha_fin
       AND (NEW.fecha_fin IS NULL OR OLD.fecha_fin IS NULL OR NEW.fecha_fin <= OLD.fecha_fin) THEN
        RAISE EXCEPTION 'la prórroga lleva la fecha final más adelante' USING ERRCODE = 'V0010';
    END IF;
    IF NEW.fecha_terminacion IS NOT NULL THEN
        SELECT max(vigente_desde) INTO ultima
        FROM versiones_contrato_laboral WHERE contrato_id = NEW.id;
        IF ultima > NEW.fecha_terminacion THEN
            RAISE EXCEPTION 'el contrato tiene una versión posterior a la terminación'
                USING ERRCODE = 'V0010';
        END IF;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_proteger_contrato_laboral BEFORE UPDATE OR DELETE ON contratos_laborales
    FOR EACH ROW EXECUTE FUNCTION proteger_contrato_laboral();

-- Una versión nueva cae dentro del contrato, después de la última, y respeta el tipo.
CREATE FUNCTION validar_version_contrato() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    c       contratos_laborales%ROWTYPE;
    ultima  DATE;
BEGIN
    SELECT * INTO c FROM contratos_laborales WHERE id = NEW.contrato_id FOR UPDATE;
    IF c.fecha_terminacion IS NOT NULL THEN
        RAISE EXCEPTION 'el contrato ya terminó' USING ERRCODE = 'V0010';
    END IF;
    IF NEW.vigente_desde < c.fecha_inicio
       OR (c.fecha_fin IS NOT NULL AND NEW.vigente_desde > c.fecha_fin) THEN
        RAISE EXCEPTION 'la versión empieza fuera de las fechas del contrato'
            USING ERRCODE = 'V0010';
    END IF;
    SELECT max(vigente_desde) INTO ultima
    FROM versiones_contrato_laboral WHERE contrato_id = NEW.contrato_id;
    IF ultima IS NULL AND NEW.vigente_desde <> c.fecha_inicio THEN
        RAISE EXCEPTION 'la primera versión empieza el día del inicio del contrato'
            USING ERRCODE = 'V0010';
    END IF;
    IF ultima IS NOT NULL AND NEW.vigente_desde <= ultima THEN
        RAISE EXCEPTION 'la versión nueva empieza después de la última' USING ERRCODE = 'V0010';
    END IF;
    IF (c.tipo = 'aprendizaje') <> (NEW.tipo_salario = 'apoyo_sostenimiento') THEN
        RAISE EXCEPTION 'el contrato de aprendizaje y el apoyo de sostenimiento van juntos'
            USING ERRCODE = 'V0010';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_validar_version_contrato BEFORE INSERT ON versiones_contrato_laboral
    FOR EACH ROW EXECUTE FUNCTION validar_version_contrato();
CREATE TRIGGER versiones_contrato_laboral_solo_insercion BEFORE UPDATE OR DELETE
    ON versiones_contrato_laboral FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER versiones_contrato_laboral_sin_truncate BEFORE TRUNCATE
    ON versiones_contrato_laboral FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

-- Todo contrato nace con su primera versión (en la misma transacción).
CREATE FUNCTION exigir_version_contrato() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM versiones_contrato_laboral WHERE contrato_id = NEW.id) THEN
        RAISE EXCEPTION 'el contrato laboral necesita su primera versión' USING ERRCODE = 'V0010';
    END IF;
    RETURN NULL;
END
$$;
CREATE CONSTRAINT TRIGGER trg_contrato_con_version
    AFTER INSERT ON contratos_laborales DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION exigir_version_contrato();

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['empleados', 'contratos_laborales',
                             'versiones_contrato_laboral'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT ON empleados, versiones_contrato_laboral TO dad_app;
GRANT SELECT, INSERT, UPDATE ON contratos_laborales TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('nomina.ver', 'copropiedad'),
    ('nomina.salarios.ver', 'copropiedad'),
    ('nomina.empleados.gestionar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r,
     unnest(ARRAY['nomina.ver', 'nomina.salarios.ver', 'nomina.empleados.gestionar']) AS p;
