-- Copropiedades y bienes privados (plan 03, ADR 0006).
--
-- Garantías que da la propia base de datos:
--   1. Un solo cuadro de coeficientes `vigente` por copropiedad (índice único parcial) y su
--      suma es exactamente 1, sin tolerancia (trigger al activarlo).
--   2. `bienes_privados.coeficiente` = coeficiente del cuadro vigente: solo lo escribe la
--      activación del cuadro (trigger), nunca una edición suelta.
--   3. Los coeficientes de un cuadro solo cambian mientras es `borrador`.
--   4. Titulares vigentes de un bien privado: suman 100 % por clase de derecho, un solo
--      representante ante la copropiedad y sin solapes con los anteriores (trigger diferido).
--   5. No se activa un cuadro con una reunión `instalada` (las votaciones en curso usan el
--      coeficiente copiado en la participación).

-- ---------------------------------------------------------------------
-- Copropiedad (tenant): identificación, ubicación, contacto, datos legales y estado
-- ---------------------------------------------------------------------
ALTER TABLE copropiedades
    ADD COLUMN razon_social           TEXT CHECK (char_length(razon_social) BETWEEN 1 AND 200),
    ADD COLUMN tipo                   TEXT NOT NULL DEFAULT 'residencial'
                                      CHECK (tipo IN ('residencial', 'comercial', 'mixta')),
    ADD COLUMN forma                  TEXT NOT NULL DEFAULT 'conjunto'
                                      CHECK (forma IN ('edificio', 'conjunto', 'condominio',
                                                       'centro_comercial', 'parcelacion', 'otro')),
    ADD COLUMN direccion              TEXT CHECK (char_length(direccion) <= 200),
    ADD COLUMN municipio_dane         TEXT CHECK (municipio_dane ~ '^[0-9]{5}$'),
    ADD COLUMN departamento           TEXT CHECK (char_length(departamento) <= 80),
    ADD COLUMN barrio                 TEXT CHECK (char_length(barrio) <= 120),
    ADD COLUMN latitud                NUMERIC(9,6) CHECK (latitud BETWEEN -90 AND 90),
    ADD COLUMN longitud               NUMERIC(9,6) CHECK (longitud BETWEEN -180 AND 180),
    ADD COLUMN estrato                SMALLINT CHECK (estrato BETWEEN 1 AND 6),
    ADD COLUMN telefono               TEXT CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    ADD COLUMN correo                 TEXT CHECK (correo = lower(correo) AND correo LIKE '%_@_%'),
    ADD COLUMN sitio_web              TEXT CHECK (sitio_web ~ '^https?://'),
    ADD COLUMN escritura_numero       TEXT CHECK (char_length(escritura_numero) <= 40),
    ADD COLUMN notaria                TEXT CHECK (char_length(notaria) <= 120),
    ADD COLUMN escritura_fecha        DATE,
    ADD COLUMN matricula_matriz       TEXT CHECK (char_length(matricula_matriz) <= 40),
    ADD COLUMN reglamento_archivo_id  UUID,
    ADD COLUMN certificado_archivo_id UUID,
    ADD COLUMN zona_horaria           TEXT NOT NULL DEFAULT 'America/Bogota',
    ADD COLUMN estado                 TEXT NOT NULL DEFAULT 'en_configuracion'
                                      CHECK (estado IN ('en_configuracion', 'activa',
                                                        'suspendida', 'retirada')),
    -- Nomenclatura de los bienes privados (domain::PlantillaNomenclatura): "{agrupacion}-{numero}"
    -- da "T3-506"; "{agrupacion}{numero}" da "3506". Un bien privado puede traer su código propio.
    ADD COLUMN plantilla_codigo       TEXT NOT NULL DEFAULT '{agrupacion}-{numero}'
                                      CHECK (char_length(plantilla_codigo) BETWEEN 8 AND 60
                                             AND plantilla_codigo LIKE '%{numero}%'),
    ADD COLUMN actualizado_en         TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD CONSTRAINT ck_copropiedades_nit CHECK (nit ~ '^[1-9][0-9]{0,14}-[0-9]$'),
    ADD CONSTRAINT fk_copropiedades_reglamento
        FOREIGN KEY (id, reglamento_archivo_id) REFERENCES archivos (tenant_id, id),
    ADD CONSTRAINT fk_copropiedades_certificado
        FOREIGN KEY (id, certificado_archivo_id) REFERENCES archivos (tenant_id, id);

-- ---------------------------------------------------------------------
-- Cargos vigentes (ficha de la PH): administrador, contador, revisor fiscal, consejo…
-- Son asignaciones de rol con vigencia (plan 02), no columnas de `copropiedades`. El nombre
-- viene de `cuentas` (plataforma): función SECURITY DEFINER restringida al tenant actual.
-- ---------------------------------------------------------------------
CREATE FUNCTION cargos_vigentes()
RETURNS TABLE (rol TEXT, rol_nombre TEXT, cuenta_id UUID, nombre TEXT, organizacion TEXT,
               vigente_desde TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT r.codigo, r.nombre, a.cuenta_id, c.nombre_mostrar, o.razon_social, a.vigente_desde
    FROM asignaciones_rol a
    JOIN roles r   ON r.id = a.rol_id
    JOIN cuentas c ON c.id = a.cuenta_id
    LEFT JOIN contratos_servicio cs
           ON a.origen = 'contrato_servicio' AND cs.id = a.origen_id AND cs.tenant_id = a.tenant_id
    LEFT JOIN organizaciones o ON o.id = cs.organizacion_id
    WHERE a.tenant_id = app_tenant_id()
      AND r.es_plantilla
      AND r.codigo IN ('administrador', 'contador', 'revisor_fiscal', 'presidente_consejo',
                       'consejero', 'comite_convivencia')
      AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
    ORDER BY array_position(ARRAY['administrador', 'contador', 'revisor_fiscal',
                                  'presidente_consejo', 'consejero', 'comite_convivencia'], r.codigo),
             c.nombre_mostrar
$$;

-- ---------------------------------------------------------------------
-- Puntos de atención (oficina de administración, portería…) y su horario semanal
-- ---------------------------------------------------------------------
CREATE TABLE puntos_atencion (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    tipo       TEXT        NOT NULL CHECK (tipo IN ('oficina_administracion', 'porteria',
                                                    'recepcion', 'otro')),
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    ubicacion  TEXT        CHECK (char_length(ubicacion) <= 200),
    telefono   TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id)
);

-- Franjas semanales (domain::horario). dia_semana ISO: 1 = lunes … 7 = domingo.
CREATE TABLE franjas_punto_atencion (
    tenant_id          UUID     NOT NULL REFERENCES copropiedades (id),
    punto_atencion_id  UUID     NOT NULL,
    dia_semana         SMALLINT NOT NULL CHECK (dia_semana BETWEEN 1 AND 7),
    hora_inicio        TIME     NOT NULL,
    hora_fin           TIME     NOT NULL CHECK (hora_fin > hora_inicio),
    aplica_festivos    BOOLEAN  NOT NULL DEFAULT FALSE,
    PRIMARY KEY (punto_atencion_id, dia_semana, hora_inicio),
    FOREIGN KEY (tenant_id, punto_atencion_id) REFERENCES puntos_atencion (tenant_id, id)
        ON DELETE CASCADE
);

-- Excepciones por fecha: cerrado (sin horas) o con una franja especial.
CREATE TABLE excepciones_punto_atencion (
    tenant_id          UUID NOT NULL REFERENCES copropiedades (id),
    punto_atencion_id  UUID NOT NULL,
    fecha              DATE NOT NULL,
    hora_inicio        TIME,
    hora_fin           TIME,
    motivo             TEXT CHECK (char_length(motivo) <= 200),
    PRIMARY KEY (punto_atencion_id, fecha),
    CHECK ((hora_inicio IS NULL) = (hora_fin IS NULL)),
    CHECK (hora_fin IS NULL OR hora_fin > hora_inicio),
    FOREIGN KEY (tenant_id, punto_atencion_id) REFERENCES puntos_atencion (tenant_id, id)
        ON DELETE CASCADE
);

-- ---------------------------------------------------------------------
-- Agrupaciones (edificio, etapa, bloque, manzana…): árbol de hasta 3 niveles
-- ---------------------------------------------------------------------
CREATE TABLE agrupaciones (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    tipo       TEXT        NOT NULL CHECK (tipo IN ('edificio', 'etapa', 'bloque', 'manzana',
                                                    'interior', 'sector')),
    -- Etiqueta para mostrar ("Torre 3") y código corto para la nomenclatura ("T3" o "3").
    nombre     TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    codigo     TEXT        NOT NULL CHECK (codigo ~ '^[A-Za-z0-9]{1,10}$'),
    padre_id   UUID,
    orden      INTEGER     NOT NULL DEFAULT 0,
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    UNIQUE NULLS NOT DISTINCT (tenant_id, padre_id, codigo),
    CHECK (padre_id IS DISTINCT FROM id),
    FOREIGN KEY (tenant_id, padre_id) REFERENCES agrupaciones (tenant_id, id)
);
CREATE INDEX ix_agrupaciones_padre ON agrupaciones (tenant_id, padre_id);

-- ---------------------------------------------------------------------
-- Bienes privados: tipo, ubicación en el árbol, datos registrales y vínculos
-- ---------------------------------------------------------------------
ALTER TABLE bienes_privados
    -- Sin cuadro vigente un bien privado aún no tiene coeficiente (y no puede votar: la
    -- participación exige uno).
    ALTER COLUMN coeficiente DROP NOT NULL,
    ADD COLUMN tipo                    TEXT NOT NULL DEFAULT 'apartamento'
                                       CHECK (tipo IN ('apartamento', 'casa', 'local', 'oficina',
                                                       'consultorio', 'parqueadero', 'deposito',
                                                       'bodega', 'lote', 'finca', 'otro')),
    ADD COLUMN agrupacion_id           UUID,
    ADD COLUMN numero                  TEXT CHECK (numero ~ '^[A-Za-z0-9-]{1,20}$'),
    ADD COLUMN piso                    SMALLINT CHECK (piso BETWEEN -10 AND 200),
    ADD COLUMN area_privada_m2         NUMERIC(10,2) CHECK (area_privada_m2 > 0),
    ADD COLUMN matricula_inmobiliaria  TEXT CHECK (char_length(matricula_inmobiliaria) <= 40),
    ADD COLUMN cedula_catastral        TEXT CHECK (char_length(cedula_catastral) <= 40),
    ADD COLUMN uso                     TEXT NOT NULL DEFAULT 'residencial'
                                       CHECK (uso IN ('residencial', 'comercial', 'mixto',
                                                      'industrial', 'institucional', 'otro')),
    -- Parqueadero o depósito con folio propio asociado a un apartamento: conserva su coeficiente.
    ADD COLUMN bien_principal_id       UUID,
    ADD COLUMN actualizado_en          TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD CONSTRAINT ck_bien_principal_distinto CHECK (bien_principal_id IS DISTINCT FROM id),
    ADD CONSTRAINT fk_bienes_privados_agrupacion
        FOREIGN KEY (tenant_id, agrupacion_id) REFERENCES agrupaciones (tenant_id, id),
    ADD CONSTRAINT fk_bienes_privados_principal
        FOREIGN KEY (tenant_id, bien_principal_id) REFERENCES bienes_privados (tenant_id, id);
CREATE INDEX ix_bienes_privados_agrupacion ON bienes_privados (tenant_id, agrupacion_id);
CREATE INDEX ix_bienes_privados_principal ON bienes_privados (tenant_id, bien_principal_id)
    WHERE bien_principal_id IS NOT NULL;
COMMENT ON COLUMN bienes_privados.matricula_inmobiliaria
    IS 'dato_personal: junto con el titular identifica su patrimonio';
COMMENT ON COLUMN bienes_privados.cedula_catastral
    IS 'dato_personal: junto con el titular identifica su patrimonio';

-- Bienes comunes de uso exclusivo (art. 22): comunes, sin coeficiente propio, asignados a un
-- bien privado. El vínculo con el catálogo de bienes comunes llega con el plan 04.
CREATE TABLE usos_exclusivos (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id  UUID        NOT NULL,
    tipo             TEXT        NOT NULL CHECK (tipo IN ('parqueadero', 'deposito', 'terraza',
                                                          'patio', 'jardin', 'cubierta', 'otro')),
    codigo           TEXT        NOT NULL CHECK (char_length(codigo) BETWEEN 1 AND 40),
    descripcion      TEXT        CHECK (char_length(descripcion) <= 200),
    vigente_desde    TIMESTAMPTZ NOT NULL,
    vigente_hasta    TIMESTAMPTZ CHECK (vigente_hasta >= vigente_desde),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);
-- Un bien común solo está asignado a un bien privado a la vez.
CREATE UNIQUE INDEX ux_usos_exclusivos_vigentes ON usos_exclusivos (tenant_id, tipo, codigo)
    WHERE vigente_hasta IS NULL;
CREATE INDEX ix_usos_exclusivos_bien ON usos_exclusivos (tenant_id, bien_privado_id);

-- ---------------------------------------------------------------------
-- Cuadros de coeficientes versionados
-- ---------------------------------------------------------------------
CREATE TABLE cuadros_coeficientes (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    version             INTEGER     NOT NULL CHECK (version > 0),
    -- `modulo_contribucion` queda previsto para la contabilidad de conjuntos mixtos (art. 29);
    -- el de votaciones y expensas es siempre `copropiedad`.
    tipo                TEXT        NOT NULL DEFAULT 'copropiedad'
                        CHECK (tipo IN ('copropiedad', 'modulo_contribucion')),
    estado              TEXT        NOT NULL DEFAULT 'borrador'
                        CHECK (estado IN ('borrador', 'vigente', 'historico')),
    descripcion         TEXT        CHECK (char_length(descripcion) <= 300),
    -- Soporte de la reforma (escritura o acta): opcional.
    soporte_archivo_id  UUID,
    vigente_desde       TIMESTAMPTZ,
    aprobado_por        UUID,       -- cuenta que lo activó
    creado_en           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, tipo, version),
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'borrador') = (vigente_desde IS NULL)),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_cuadro_vigente ON cuadros_coeficientes (tenant_id, tipo)
    WHERE estado = 'vigente';

CREATE TABLE coeficientes_bien_privado (
    tenant_id        UUID          NOT NULL REFERENCES copropiedades (id),
    cuadro_id        UUID          NOT NULL,
    bien_privado_id  UUID          NOT NULL,
    coeficiente      NUMERIC(10,8) NOT NULL CHECK (coeficiente > 0 AND coeficiente <= 1),
    PRIMARY KEY (cuadro_id, bien_privado_id),
    FOREIGN KEY (tenant_id, cuadro_id)       REFERENCES cuadros_coeficientes (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);
CREATE INDEX ix_coeficientes_bien ON coeficientes_bien_privado (tenant_id, bien_privado_id);

-- Los coeficientes de un cuadro activado o histórico son la historia legal: inmutables.
CREATE FUNCTION exigir_cuadro_borrador() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_estado TEXT;
BEGIN
    SELECT estado INTO v_estado FROM cuadros_coeficientes
    WHERE id = coalesce(NEW.cuadro_id, OLD.cuadro_id)
    FOR SHARE;
    IF v_estado IS DISTINCT FROM 'borrador' THEN
        RAISE EXCEPTION 'el cuadro de coeficientes no es un borrador (estado: %)',
            coalesce(v_estado, 'inexistente')
            USING ERRCODE = 'V0002';
    END IF;
    RETURN coalesce(NEW, OLD);
END
$$;
CREATE TRIGGER trg_coeficientes_solo_borrador
    BEFORE INSERT OR UPDATE OR DELETE ON coeficientes_bien_privado
    FOR EACH ROW EXECUTE FUNCTION exigir_cuadro_borrador();

-- Ciclo de vida: borrador → vigente → historico. Al pasar a vigente: suma exacta = 1, todos los
-- bienes privados activos incluidos y ninguna reunión instalada.
CREATE FUNCTION validar_cuadro_coeficientes() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_suma      NUMERIC;
    v_faltantes INTEGER;
BEGIN
    IF NEW.estado = OLD.estado THEN
        IF NEW.estado <> 'borrador' THEN
            RAISE EXCEPTION 'un cuadro % no se modifica', OLD.estado USING ERRCODE = 'V0002';
        END IF;
        RETURN NEW;
    END IF;
    IF NOT ((OLD.estado = 'borrador' AND NEW.estado = 'vigente')
            OR (OLD.estado = 'vigente' AND NEW.estado = 'historico')) THEN
        RAISE EXCEPTION 'transición de cuadro no permitida: % -> %', OLD.estado, NEW.estado
            USING ERRCODE = 'V0002';
    END IF;
    IF NEW.estado = 'vigente' THEN
        SELECT coalesce(sum(coeficiente), 0) INTO v_suma
        FROM coeficientes_bien_privado WHERE cuadro_id = NEW.id;
        IF v_suma <> 1 THEN
            RAISE EXCEPTION 'los coeficientes suman % y deben sumar exactamente 1', v_suma
                USING ERRCODE = 'V0003';
        END IF;
        IF NEW.tipo = 'copropiedad' THEN
            SELECT count(*) INTO v_faltantes
            FROM bienes_privados b
            WHERE b.tenant_id = NEW.tenant_id AND b.activo
              AND NOT EXISTS (SELECT 1 FROM coeficientes_bien_privado c
                              WHERE c.cuadro_id = NEW.id AND c.bien_privado_id = b.id);
            IF v_faltantes > 0 THEN
                RAISE EXCEPTION 'faltan % bienes privados activos en el cuadro', v_faltantes
                    USING ERRCODE = 'V0003';
            END IF;
            IF EXISTS (SELECT 1 FROM reuniones
                       WHERE tenant_id = NEW.tenant_id AND estado = 'instalada') THEN
                RAISE EXCEPTION 'hay una reunión instalada: no se cambian los coeficientes'
                    USING ERRCODE = 'V0004';
            END IF;
        END IF;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_cuadros_validar
    BEFORE UPDATE ON cuadros_coeficientes
    FOR EACH ROW EXECUTE FUNCTION validar_cuadro_coeficientes();

-- Activado el cuadro de la copropiedad, cada bien privado toma su coeficiente (o NULL si no
-- está en él). Es la única escritura permitida de `bienes_privados.coeficiente`.
CREATE FUNCTION aplicar_cuadro_vigente() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE bienes_privados b
    SET coeficiente = c.coeficiente
    FROM (SELECT bp.id, cb.coeficiente
          FROM bienes_privados bp
          LEFT JOIN coeficientes_bien_privado cb
                 ON cb.cuadro_id = NEW.id AND cb.bien_privado_id = bp.id
          WHERE bp.tenant_id = NEW.tenant_id) c
    WHERE b.id = c.id AND b.coeficiente IS DISTINCT FROM c.coeficiente;
    RETURN NULL;
END
$$;
CREATE TRIGGER trg_cuadros_aplicar
    AFTER UPDATE OF estado ON cuadros_coeficientes
    FOR EACH ROW WHEN (NEW.estado = 'vigente' AND NEW.tipo = 'copropiedad')
    EXECUTE FUNCTION aplicar_cuadro_vigente();

-- `bienes_privados.coeficiente` solo puede quedar igual al del cuadro vigente.
CREATE FUNCTION exigir_coeficiente_del_cuadro() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_vigente NUMERIC;
BEGIN
    SELECT cb.coeficiente INTO v_vigente
    FROM cuadros_coeficientes q
    JOIN coeficientes_bien_privado cb ON cb.cuadro_id = q.id AND cb.bien_privado_id = NEW.id
    WHERE q.tenant_id = NEW.tenant_id AND q.tipo = 'copropiedad' AND q.estado = 'vigente';
    IF NEW.coeficiente IS DISTINCT FROM v_vigente THEN
        RAISE EXCEPTION 'el coeficiente de un bien privado solo cambia activando un cuadro'
            USING ERRCODE = 'V0002';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_bienes_privados_coeficiente
    BEFORE UPDATE OF coeficiente ON bienes_privados
    FOR EACH ROW WHEN (NEW.coeficiente IS DISTINCT FROM OLD.coeficiente)
    EXECUTE FUNCTION exigir_coeficiente_del_cuadro();

-- ---------------------------------------------------------------------
-- Personas: cuenta global y personas jurídicas
-- ---------------------------------------------------------------------
ALTER TABLE personas
    ADD COLUMN tipo                    TEXT NOT NULL DEFAULT 'natural'
                                       CHECK (tipo IN ('natural', 'juridica')),
    ADD COLUMN cuenta_id               UUID REFERENCES cuentas (id),
    -- Quien actúa por una persona jurídica.
    ADD COLUMN representante_legal_id  UUID,
    ADD CONSTRAINT uq_personas_cuenta UNIQUE (tenant_id, cuenta_id),
    ADD CONSTRAINT ck_representante_legal CHECK (
        representante_legal_id IS NULL OR (tipo = 'juridica' AND representante_legal_id <> id)),
    ADD CONSTRAINT fk_personas_representante_legal
        FOREIGN KEY (tenant_id, representante_legal_id) REFERENCES personas (tenant_id, id);

-- ---------------------------------------------------------------------
-- Titularidad de los bienes privados
-- ---------------------------------------------------------------------
CREATE TABLE titularidades (
    id                  UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL REFERENCES copropiedades (id),
    bien_privado_id     UUID         NOT NULL,
    persona_id          UUID         NOT NULL,
    tipo                TEXT         NOT NULL CHECK (tipo IN ('propietario', 'nudo_propietario',
                                                              'usufructuario', 'fideicomiso',
                                                              'leasing')),
    porcentaje          NUMERIC(7,4) NOT NULL CHECK (porcentaje > 0 AND porcentaje <= 100),
    -- Quien representa al bien privado ante la copropiedad (votos, notificaciones).
    es_representante    BOOLEAN      NOT NULL DEFAULT FALSE,
    vigente_desde       TIMESTAMPTZ  NOT NULL,
    vigente_hasta       TIMESTAMPTZ  CHECK (vigente_hasta >= vigente_desde),
    soporte_archivo_id  UUID,
    creado_en           TIMESTAMPTZ  NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)    REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)         REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_titularidad_vigente ON titularidades (bien_privado_id, persona_id, tipo)
    WHERE vigente_hasta IS NULL;
CREATE UNIQUE INDEX ux_representante_vigente ON titularidades (bien_privado_id)
    WHERE vigente_hasta IS NULL AND es_representante;
CREATE INDEX ix_titularidades_bien ON titularidades (tenant_id, bien_privado_id, vigente_desde);
CREATE INDEX ix_titularidades_persona ON titularidades (tenant_id, persona_id);

-- Al final de la transacción: los titulares vigentes de cada bien privado tocado suman 100 %
-- por clase de derecho (dominio: propietario, nudo propietario, fideicomiso, leasing; goce:
-- usufructuario), tienen exactamente un representante y no se solapan con los anteriores.
CREATE FUNCTION validar_titulares_bien() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_bien      UUID := coalesce(NEW.bien_privado_id, OLD.bien_privado_id);
    v_abiertos  INTEGER;
    v_repr      INTEGER;
    v_desde     TIMESTAMPTZ;
BEGIN
    SELECT count(*), count(*) FILTER (WHERE es_representante), min(vigente_desde)
    INTO v_abiertos, v_repr, v_desde
    FROM titularidades WHERE bien_privado_id = v_bien AND vigente_hasta IS NULL;
    IF v_abiertos = 0 THEN
        RETURN NULL;
    END IF;
    IF EXISTS (
        SELECT 1 FROM titularidades
        WHERE bien_privado_id = v_bien AND vigente_hasta IS NULL
        GROUP BY (tipo = 'usufructuario')
        HAVING sum(porcentaje) <> 100
    ) THEN
        RAISE EXCEPTION 'los titulares vigentes del bien privado % no suman 100 %%', v_bien
            USING ERRCODE = 'V0005';
    END IF;
    IF v_repr <> 1 THEN
        RAISE EXCEPTION 'el bien privado % debe tener exactamente un representante', v_bien
            USING ERRCODE = 'V0005';
    END IF;
    IF EXISTS (SELECT 1 FROM titularidades
               WHERE bien_privado_id = v_bien AND vigente_hasta > v_desde) THEN
        RAISE EXCEPTION 'la titularidad nueva del bien privado % se solapa con la anterior', v_bien
            USING ERRCODE = 'V0005';
    END IF;
    RETURN NULL;
END
$$;
CREATE CONSTRAINT TRIGGER trg_titularidades_validar
    AFTER INSERT OR UPDATE ON titularidades
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION validar_titulares_bien();

-- ---------------------------------------------------------------------
-- Importaciones masivas (archivo CSV/XLSX → validación → vista previa → aplicar)
-- ---------------------------------------------------------------------
ALTER TABLE archivos DROP CONSTRAINT archivos_tipo_mime_check;
ALTER TABLE archivos ADD CONSTRAINT archivos_tipo_mime_check CHECK (tipo_mime IN (
    'application/pdf', 'image/jpeg', 'image/png', 'image/webp', 'image/heic', 'text/csv',
    'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'));

CREATE TABLE importaciones (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES copropiedades (id),
    tipo          TEXT        NOT NULL CHECK (tipo IN ('bienes_privados')),
    archivo_id    UUID        NOT NULL,
    estado        TEXT        NOT NULL CHECK (estado IN ('validada', 'con_errores', 'aplicada')),
    total_filas   INTEGER     NOT NULL CHECK (total_filas >= 0),
    filas_validas INTEGER     NOT NULL CHECK (filas_validas BETWEEN 0 AND total_filas),
    -- [{ "fila": 3, "columna": "coeficiente", "mensaje": "…" }]. Los mensajes nunca repiten el
    -- valor de la celda (puede ser un documento o un teléfono).
    errores       JSONB       NOT NULL DEFAULT '[]'::jsonb,
    resumen       JSONB       NOT NULL DEFAULT '{}'::jsonb,
    creada_por    UUID,
    creada_en     TIMESTAMPTZ NOT NULL,
    aplicada_en   TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'aplicada') = (aplicada_en IS NOT NULL)),
    FOREIGN KEY (tenant_id, archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_importaciones_fecha ON importaciones (tenant_id, creada_en DESC);

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'puntos_atencion', 'franjas_punto_atencion', 'excepciones_punto_atencion', 'agrupaciones',
        'usos_exclusivos', 'cuadros_coeficientes', 'coeficientes_bien_privado', 'titularidades',
        'importaciones'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT, UPDATE, DELETE ON puntos_atencion, franjas_punto_atencion,
    excepciones_punto_atencion TO dad_app;
GRANT SELECT, INSERT ON agrupaciones TO dad_app;
GRANT UPDATE (nombre, codigo, orden) ON agrupaciones TO dad_app;
GRANT SELECT, INSERT ON usos_exclusivos TO dad_app;
GRANT UPDATE (vigente_hasta, descripcion) ON usos_exclusivos TO dad_app;
GRANT SELECT, INSERT ON cuadros_coeficientes TO dad_app;
GRANT UPDATE (estado, descripcion, soporte_archivo_id, vigente_desde, aprobado_por)
    ON cuadros_coeficientes TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON coeficientes_bien_privado TO dad_app;
GRANT SELECT, INSERT ON titularidades TO dad_app;
GRANT UPDATE (vigente_hasta, es_representante) ON titularidades TO dad_app;
GRANT SELECT, INSERT ON importaciones TO dad_app;
GRANT UPDATE (estado, resumen, aplicada_en) ON importaciones TO dad_app;

REVOKE ALL ON FUNCTION cargos_vigentes() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cargos_vigentes() TO dad_app;

-- ---------------------------------------------------------------------
-- Catálogo de permisos (= domain::Permiso) y plantillas
-- ---------------------------------------------------------------------
INSERT INTO permisos (codigo, ambito) VALUES
    ('agrupaciones.editar', 'copropiedad'),
    ('bienes_privados.coeficientes.activar', 'copropiedad'),
    ('titulares.ver', 'copropiedad'),
    ('titulares.editar', 'copropiedad'),
    ('importaciones.ejecutar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'agrupaciones.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_privados.coeficientes.activar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'titulares.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'titulares.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'importaciones.ejecutar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'titulares.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'titulares.ver', NULL);
