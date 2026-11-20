-- Liquidación de la nómina (plan 16 fase 2, ADR 0025): tablas por tramos del catálogo laboral
-- (retención en la fuente y fondo de solidaridad), configuración de la entidad, periodos,
-- novedades y liquidaciones.
--
-- Garantías de la base de datos:
--   1. Las tablas por tramos y las novedades son de solo inserción; una novedad se anula con un
--      registro aparte.
--   2. Un periodo aprobado o pagado no cambia: ni sus liquidaciones ni sus líneas, ni novedades
--      nuevas o anulaciones en sus fechas.
--   3. Aprueba una cuenta distinta de la que liquidó.
-- Los errores de regla laboral usan el código `V0010`.

-- ---------------------------------------------------------------------
-- Catálogo: parámetros nuevos y tablas por tramos
-- ---------------------------------------------------------------------
ALTER TABLE parametros_laborales DROP CONSTRAINT parametros_laborales_parametro_check;
ALTER TABLE parametros_laborales ADD CONSTRAINT parametros_laborales_parametro_check
    CHECK (parametro IN (
        'salario_minimo', 'auxilio_transporte', 'tope_auxilio_transporte',
        'jornada_maxima_semanal', 'inicio_jornada_nocturna', 'fin_jornada_nocturna',
        'recargo_nocturno', 'hora_extra_diurna', 'hora_extra_nocturna',
        'recargo_dominical_festivo', 'salud_empleador', 'salud_empleado', 'pension_empleador',
        'pension_empleado', 'caja_compensacion', 'icbf', 'sena', 'arl_clase_1', 'arl_clase_2',
        'arl_clase_3', 'arl_clase_4', 'arl_clase_5', 'salario_integral_minimo',
        'factor_salario_integral', 'limite_pagos_no_salariales', 'dias_incapacidad_empleador',
        'incapacidad_eps', 'dias_cesantias', 'intereses_cesantias', 'dias_prima',
        'dias_vacaciones', 'apoyo_aprendiz_lectiva', 'apoyo_aprendiz_productiva', 'tope_ibc',
        'tope_exoneracion_aportes', 'renta_exenta_laboral', 'tope_renta_exenta_laboral',
        'limite_beneficios_retencion', 'tope_beneficios_retencion'));

-- Una versión de una tabla = todas sus filas con el mismo `vigente_desde`.
CREATE TABLE tramos_laborales (
    id             UUID          PRIMARY KEY,
    tabla          TEXT          NOT NULL CHECK (tabla IN ('retencion_salarios', 'fondo_solidaridad')),
    vigente_desde  DATE          NOT NULL,
    desde          NUMERIC(14,4) NOT NULL CHECK (desde >= 0),
    hasta          NUMERIC(14,4) CHECK (hasta > desde),
    -- Porcentaje.
    tarifa         NUMERIC(7,4)  NOT NULL CHECK (tarifa >= 0 AND tarifa <= 100),
    adicional      NUMERIC(14,4) NOT NULL DEFAULT 0 CHECK (adicional >= 0),
    fuente         TEXT          NOT NULL CHECK (char_length(fuente) BETWEEN 3 AND 300),
    creado_por     UUID,
    creado_en      TIMESTAMPTZ   NOT NULL,
    UNIQUE (tabla, vigente_desde, desde)
);
ALTER TABLE tramos_laborales ENABLE ROW LEVEL SECURITY;
CREATE POLICY ver ON tramos_laborales FOR SELECT USING (TRUE);
CREATE POLICY crear ON tramos_laborales FOR INSERT WITH CHECK (app_plataforma());
CREATE TRIGGER tramos_laborales_solo_insercion BEFORE UPDATE OR DELETE ON tramos_laborales
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER tramos_laborales_sin_truncate BEFORE TRUNCATE ON tramos_laborales
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();
GRANT SELECT, INSERT ON tramos_laborales TO dad_app;

-- ---------------------------------------------------------------------
-- Configuración de la nómina de la entidad
-- ---------------------------------------------------------------------
CREATE TABLE configuraciones_nomina (
    tenant_id       UUID        PRIMARY KEY REFERENCES entidades (id),
    -- Aplica la exoneración del art. 114-1 del ET (salud del empleador, ICBF y SENA).
    exonerada_aportes BOOLEAN   NOT NULL DEFAULT FALSE,
    actualizada_por UUID,
    actualizada_en  TIMESTAMPTZ NOT NULL
);

-- ---------------------------------------------------------------------
-- Periodos, novedades y liquidaciones
-- ---------------------------------------------------------------------
CREATE TABLE periodos_nomina (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES entidades (id),
    periodicidad   TEXT        NOT NULL CHECK (periodicidad IN ('mensual', 'quincenal')),
    desde          DATE        NOT NULL,
    hasta          DATE        NOT NULL CHECK (hasta >= desde),
    estado         TEXT        NOT NULL DEFAULT 'abierto'
                   CHECK (estado IN ('abierto', 'liquidado', 'aprobado', 'pagado')),
    creado_por     UUID,
    creado_en      TIMESTAMPTZ NOT NULL,
    liquidado_por  UUID,
    liquidado_en   TIMESTAMPTZ,
    aprobado_por   UUID,
    aprobado_en    TIMESTAMPTZ,
    pagado_por     UUID,
    pagado_en      TIMESTAMPTZ,
    fecha_pago     DATE,
    CONSTRAINT ck_periodo_liquidado
        CHECK ((estado = 'abierto') = (liquidado_en IS NULL)),
    CONSTRAINT ck_periodo_aprobado
        CHECK ((estado IN ('aprobado', 'pagado')) = (aprobado_en IS NOT NULL)),
    CONSTRAINT ck_periodo_pagado CHECK ((estado = 'pagado') = (fecha_pago IS NOT NULL)),
    -- Cuatro ojos: aprueba otra cuenta.
    CONSTRAINT ck_periodo_otra_cuenta CHECK (aprobado_por IS DISTINCT FROM liquidado_por
                                             OR aprobado_por IS NULL),
    UNIQUE (tenant_id, periodicidad, desde),
    UNIQUE (tenant_id, id)
);

-- Un periodo aprobado o pagado no vuelve atrás ni cambia de fechas.
CREATE FUNCTION proteger_periodo_nomina() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'un periodo de nómina no se borra' USING ERRCODE = 'V0010';
    END IF;
    IF NEW.periodicidad <> OLD.periodicidad OR NEW.desde <> OLD.desde OR NEW.hasta <> OLD.hasta
    THEN
        RAISE EXCEPTION 'las fechas de un periodo no cambian' USING ERRCODE = 'V0010';
    END IF;
    IF OLD.estado = 'pagado'
       OR (OLD.estado = 'aprobado' AND NEW.estado NOT IN ('aprobado', 'pagado'))
       OR (OLD.estado = 'aprobado' AND (NEW.liquidado_en IS DISTINCT FROM OLD.liquidado_en
                                        OR NEW.aprobado_en IS DISTINCT FROM OLD.aprobado_en)) THEN
        RAISE EXCEPTION 'el periodo de nómina ya fue aprobado' USING ERRCODE = 'V0010';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_proteger_periodo_nomina BEFORE UPDATE OR DELETE ON periodos_nomina
    FOR EACH ROW EXECUTE FUNCTION proteger_periodo_nomina();

CREATE TABLE novedades_nomina (
    id                  UUID          PRIMARY KEY,
    tenant_id           UUID          NOT NULL REFERENCES entidades (id),
    contrato_id         UUID          NOT NULL,
    tipo                TEXT          NOT NULL CHECK (tipo IN (
                            'hora_extra_diurna', 'hora_extra_nocturna',
                            'hora_extra_dominical_diurna', 'hora_extra_dominical_nocturna',
                            'recargo_nocturno', 'recargo_dominical', 'recargo_nocturno_dominical',
                            'incapacidad_comun', 'incapacidad_laboral', 'licencia_maternidad',
                            'licencia_paternidad', 'licencia_remunerada', 'licencia_no_remunerada',
                            'ausencia', 'suspension', 'bonificacion_salarial',
                            'bonificacion_no_salarial', 'comision', 'auxilio_no_salarial',
                            'prestamo', 'libranza', 'embargo', 'otra_deduccion')),
    desde               DATE          NOT NULL,
    hasta               DATE          NOT NULL CHECK (hasta >= desde),
    -- Horas o valor en pesos (cero en las de días).
    cantidad            NUMERIC(14,2) NOT NULL CHECK (cantidad >= 0),
    descripcion         TEXT          CHECK (char_length(descripcion) <= 300),
    soporte_archivo_id  UUID,
    creado_por          UUID,
    creado_en           TIMESTAMPTZ   NOT NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, contrato_id) REFERENCES contratos_laborales (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_novedades_contrato ON novedades_nomina (tenant_id, contrato_id, desde);
COMMENT ON COLUMN novedades_nomina.tipo IS
    'dato_sensible: puede revelar incapacidades y licencias (nunca el diagnóstico)';
COMMENT ON COLUMN novedades_nomina.descripcion IS 'dato_personal: detalle de la novedad';
COMMENT ON COLUMN novedades_nomina.cantidad IS 'dato_personal: horas o valor de la novedad';

CREATE TABLE anulaciones_novedad_nomina (
    novedad_id   UUID        PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES entidades (id),
    motivo       TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 3 AND 300),
    anulada_por  UUID,
    anulada_en   TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, novedad_id) REFERENCES novedades_nomina (tenant_id, id)
);

-- Las fechas de un periodo aprobado o pagado quedan cerradas a las novedades.
CREATE FUNCTION exigir_novedad_en_periodo_abierto() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    d DATE;
    h DATE;
BEGIN
    IF TG_TABLE_NAME = 'novedades_nomina' THEN
        d := NEW.desde;
        h := NEW.hasta;
    ELSE
        SELECT n.desde, n.hasta INTO d, h FROM novedades_nomina n WHERE n.id = NEW.novedad_id;
    END IF;
    IF EXISTS (SELECT 1 FROM periodos_nomina p
               WHERE p.estado IN ('aprobado', 'pagado') AND p.desde <= h AND p.hasta >= d) THEN
        RAISE EXCEPTION 'la novedad cae en un periodo de nómina aprobado' USING ERRCODE = 'V0010';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_novedad_periodo_abierto BEFORE INSERT ON novedades_nomina
    FOR EACH ROW EXECUTE FUNCTION exigir_novedad_en_periodo_abierto();
CREATE TRIGGER trg_anulacion_periodo_abierto BEFORE INSERT ON anulaciones_novedad_nomina
    FOR EACH ROW EXECUTE FUNCTION exigir_novedad_en_periodo_abierto();

CREATE TABLE liquidaciones_nomina (
    id                  UUID          PRIMARY KEY,
    tenant_id           UUID          NOT NULL REFERENCES entidades (id),
    periodo_id          UUID          NOT NULL,
    contrato_id         UUID          NOT NULL,
    empleado_id         UUID          NOT NULL,
    dias                SMALLINT      NOT NULL CHECK (dias BETWEEN 0 AND 30),
    ibc                 NUMERIC(18,2) NOT NULL CHECK (ibc >= 0),
    devengos            NUMERIC(18,2) NOT NULL CHECK (devengos >= 0),
    deducciones         NUMERIC(18,2) NOT NULL CHECK (deducciones >= 0),
    neto                NUMERIC(18,2) NOT NULL CHECK (neto >= 0),
    aportes_empleador   NUMERIC(18,2) NOT NULL CHECK (aportes_empleador >= 0),
    -- Para la retención de la segunda quincena.
    ingresos_mes        NUMERIC(18,2) NOT NULL,
    aportes_mes         NUMERIC(18,2) NOT NULL,
    retenido_mes        NUMERIC(18,2) NOT NULL,
    creado_en           TIMESTAMPTZ   NOT NULL,
    CONSTRAINT ck_liquidacion_neto CHECK (neto = devengos - deducciones),
    UNIQUE (periodo_id, contrato_id),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, periodo_id) REFERENCES periodos_nomina (tenant_id, id),
    FOREIGN KEY (tenant_id, contrato_id) REFERENCES contratos_laborales (tenant_id, id),
    FOREIGN KEY (tenant_id, empleado_id) REFERENCES empleados (tenant_id, id)
);
CREATE INDEX ix_liquidaciones_empleado ON liquidaciones_nomina (tenant_id, empleado_id);
COMMENT ON COLUMN liquidaciones_nomina.ibc IS 'dato_personal: ingreso base de cotización';
COMMENT ON COLUMN liquidaciones_nomina.devengos IS 'dato_personal: devengos del empleado';
COMMENT ON COLUMN liquidaciones_nomina.neto IS 'dato_personal: neto a pagar al empleado';

CREATE TABLE lineas_liquidacion_nomina (
    liquidacion_id  UUID          NOT NULL,
    tenant_id       UUID          NOT NULL REFERENCES entidades (id),
    concepto        TEXT          NOT NULL CHECK (concepto ~ '^[a-z][a-z0-9_]{1,59}$'),
    clase           TEXT          NOT NULL CHECK (clase IN ('devengo', 'deduccion', 'aporte_empleador')),
    -- Días, horas o el porcentaje del aporte.
    cantidad        NUMERIC(14,4) NOT NULL,
    valor           NUMERIC(18,2) NOT NULL CHECK (valor > 0),
    PRIMARY KEY (liquidacion_id, concepto),
    FOREIGN KEY (tenant_id, liquidacion_id) REFERENCES liquidaciones_nomina (tenant_id, id)
        ON DELETE CASCADE
);
COMMENT ON COLUMN lineas_liquidacion_nomina.valor IS 'dato_personal: valor de la línea';

-- Mientras el periodo no esté aprobado, liquidar de nuevo reemplaza sus liquidaciones.
CREATE FUNCTION exigir_periodo_sin_aprobar() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    periodo UUID;
    e       TEXT;
BEGIN
    IF TG_TABLE_NAME = 'liquidaciones_nomina' THEN
        periodo := COALESCE(NEW.periodo_id, OLD.periodo_id);
    ELSE
        SELECT l.periodo_id INTO periodo FROM liquidaciones_nomina l
        WHERE l.id = COALESCE(NEW.liquidacion_id, OLD.liquidacion_id);
    END IF;
    SELECT p.estado INTO e FROM periodos_nomina p WHERE p.id = periodo;
    IF e IN ('aprobado', 'pagado') THEN
        RAISE EXCEPTION 'el periodo de nómina ya fue aprobado' USING ERRCODE = 'V0010';
    END IF;
    RETURN COALESCE(NEW, OLD);
END
$$;
CREATE TRIGGER trg_liquidacion_sin_aprobar BEFORE INSERT OR UPDATE OR DELETE
    ON liquidaciones_nomina FOR EACH ROW EXECUTE FUNCTION exigir_periodo_sin_aprobar();
CREATE TRIGGER trg_linea_sin_aprobar BEFORE INSERT OR UPDATE OR DELETE
    ON lineas_liquidacion_nomina FOR EACH ROW EXECUTE FUNCTION exigir_periodo_sin_aprobar();

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['novedades_nomina', 'anulaciones_novedad_nomina'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion()', t || '_solo_insercion', t);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I '
            'FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion()', t || '_sin_truncate', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['configuraciones_nomina', 'periodos_nomina', 'novedades_nomina',
                             'anulaciones_novedad_nomina', 'liquidaciones_nomina',
                             'lineas_liquidacion_nomina'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;

GRANT SELECT, INSERT, UPDATE ON configuraciones_nomina, periodos_nomina TO dad_app;
GRANT SELECT, INSERT ON novedades_nomina, anulaciones_novedad_nomina TO dad_app;
GRANT SELECT, INSERT, DELETE ON liquidaciones_nomina, lineas_liquidacion_nomina TO dad_app;

-- ---------------------------------------------------------------------
-- Permisos
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('nomina.configurar', 'copropiedad'),
    ('nomina.novedades.registrar', 'copropiedad'),
    ('nomina.liquidar', 'copropiedad'),
    ('nomina.aprobar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', p, NULL
FROM unnest(ARRAY['contador', 'administrador']) AS r,
     unnest(ARRAY['nomina.configurar', 'nomina.novedades.registrar', 'nomina.liquidar',
                  'nomina.aprobar']) AS p;
