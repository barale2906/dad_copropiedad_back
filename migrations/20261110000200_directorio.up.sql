-- Directorio de comercios y servicios (plan 08, fase 1; ADR 0013).
--
-- La publicidad es un negocio de la plataforma: anunciantes, fichas y categorías son tablas de
-- plataforma (`app.plataforma = on`). Una copropiedad solo ve las fichas que la alcanzan, por
-- la función `fichas_directorio_copropiedad` (SECURITY DEFINER, tenant de la transacción).
-- La cobertura se decide con la ubicación de la PH, nunca con datos de personas.
-- Las reseñas son por copropiedad (RLS); el promedio de calificaciones suma todas las PH.

-- ---------------------------------------------------------------------
-- Categorías (árbol de dos niveles, con sinónimos para la búsqueda)
-- ---------------------------------------------------------------------
CREATE TABLE categorias_publicidad (
    codigo       TEXT    PRIMARY KEY CHECK (codigo ~ '^[a-z][a-z0-9_]{1,49}$'),
    padre        TEXT    REFERENCES categorias_publicidad (codigo),
    nombre       TEXT    NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    sinonimos    TEXT[]  NOT NULL DEFAULT '{}' CHECK (cardinality(sinonimos) <= 30),
    -- Licores, apuestas, salud con fórmula: no se pueden elegir en una ficha (plan 08).
    restringida  BOOLEAN NOT NULL DEFAULT FALSE,
    activa       BOOLEAN NOT NULL DEFAULT TRUE,
    CHECK (padre IS DISTINCT FROM codigo)
);
CREATE INDEX ix_categorias_publicidad_padre ON categorias_publicidad (padre);

-- ---------------------------------------------------------------------
-- Anunciantes y fichas del directorio
-- ---------------------------------------------------------------------
CREATE TABLE anunciantes (
    id                   UUID          PRIMARY KEY,
    organizacion_id      UUID          NOT NULL UNIQUE REFERENCES organizaciones (id),
    nombre_comercial     TEXT          NOT NULL CHECK (char_length(nombre_comercial) BETWEEN 1 AND 120),
    direccion            TEXT          CHECK (char_length(direccion) BETWEEN 1 AND 200),
    municipio_dane       TEXT          CHECK (municipio_dane ~ '^[0-9]{5}$'),
    latitud              NUMERIC(9,6)  CHECK (latitud BETWEEN -90 AND 90),
    longitud             NUMERIC(9,6)  CHECK (longitud BETWEEN -180 AND 180),
    estado_verificacion  TEXT          NOT NULL CHECK (estado_verificacion IN (
                             'pendiente', 'verificado', 'rechazado', 'suspendido')),
    motivo_estado        TEXT          CHECK (char_length(motivo_estado) BETWEEN 1 AND 500),
    creado_por           UUID          REFERENCES cuentas (id),
    creado_en            TIMESTAMPTZ   NOT NULL,
    actualizado_en       TIMESTAMPTZ   NOT NULL,
    CHECK ((latitud IS NULL) = (longitud IS NULL)),
    CHECK (estado_verificacion NOT IN ('rechazado', 'suspendido') OR motivo_estado IS NOT NULL)
);
CREATE INDEX ix_anunciantes_estado ON anunciantes (estado_verificacion, id);
CREATE INDEX ix_anunciantes_nombre_trgm ON anunciantes
    USING gin (normalizar(nombre_comercial) gin_trgm_ops);
COMMENT ON COLUMN anunciantes.direccion IS
    'dato_personal: en personas naturales (costurera, técnico) puede ser su domicilio';

CREATE TABLE fichas_directorio (
    anunciante_id    UUID        PRIMARY KEY REFERENCES anunciantes (id),
    descripcion      TEXT        CHECK (char_length(descripcion) BETWEEN 1 AND 1000),
    cobertura        TEXT        NOT NULL CHECK (cobertura IN ('nacional', 'municipio', 'radio')),
    radio_km         SMALLINT    CHECK (radio_km BETWEEN 1 AND 100),
    telefono         TEXT        CHECK (telefono ~ '^[1-9][0-9]{7,14}$'),
    whatsapp         TEXT        CHECK (whatsapp ~ '^[1-9][0-9]{7,14}$'),
    correo           TEXT        CHECK (correo = lower(correo) AND correo LIKE '%_@_%'),
    sitio_web        TEXT        CHECK (sitio_web ~ '^https://' AND char_length(sitio_web) <= 300),
    horario          TEXT        CHECK (char_length(horario) BETWEEN 1 AND 300),
    -- Ficha destacada (suscripción `directorio_destacado`): sale primero solo donde la
    -- plataforma habilitó el módulo `publicidad`.
    destacada_hasta  TIMESTAMPTZ,
    publicada        BOOLEAN     NOT NULL DEFAULT TRUE,
    actualizada_en   TIMESTAMPTZ NOT NULL,
    CHECK ((cobertura = 'radio') = (radio_km IS NOT NULL))
);
COMMENT ON COLUMN fichas_directorio.telefono IS
    'dato_personal: contacto publicado por el anunciante (persona natural)';
COMMENT ON COLUMN fichas_directorio.whatsapp IS
    'dato_personal: contacto publicado por el anunciante (persona natural)';
COMMENT ON COLUMN fichas_directorio.correo IS
    'dato_personal: contacto publicado por el anunciante (persona natural)';

CREATE TABLE ficha_categorias (
    anunciante_id  UUID     NOT NULL REFERENCES fichas_directorio (anunciante_id) ON DELETE CASCADE,
    categoria      TEXT     NOT NULL REFERENCES categorias_publicidad (codigo),
    -- 0 = principal.
    orden          SMALLINT NOT NULL CHECK (orden BETWEEN 0 AND 4),
    PRIMARY KEY (anunciante_id, categoria),
    UNIQUE (anunciante_id, orden)
);
CREATE INDEX ix_ficha_categorias_categoria ON ficha_categorias (categoria);

-- Invariante 9 del plan 08: una ficha nunca usa una categoría restringida (también lo valida
-- el dominio; esto protege de un INSERT directo).
CREATE FUNCTION rechazar_categoria_restringida() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM categorias_publicidad
               WHERE codigo = NEW.categoria AND (restringida OR NOT activa)) THEN
        RAISE EXCEPTION 'categoría restringida o inactiva: %', NEW.categoria
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER ficha_sin_categoria_restringida
    BEFORE INSERT OR UPDATE ON ficha_categorias
    FOR EACH ROW EXECUTE FUNCTION rechazar_categoria_restringida();

-- ---------------------------------------------------------------------
-- Reseñas (por copropiedad) y sus reportes
-- ---------------------------------------------------------------------
CREATE TABLE resenas (
    id                   UUID        PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES copropiedades (id),
    anunciante_id        UUID        NOT NULL REFERENCES anunciantes (id),
    cuenta_id            UUID        NOT NULL REFERENCES cuentas (id),
    calificacion         SMALLINT    NOT NULL CHECK (calificacion BETWEEN 1 AND 5),
    texto                TEXT        CHECK (char_length(texto) BETWEEN 1 AND 1000),
    estado               TEXT        NOT NULL DEFAULT 'publicada'
                         CHECK (estado IN ('publicada', 'oculta')),
    motivo_moderacion    TEXT        CHECK (char_length(motivo_moderacion) BETWEEN 1 AND 500),
    moderada_por         UUID,
    moderada_en          TIMESTAMPTZ,
    creada_en            TIMESTAMPTZ NOT NULL,
    actualizada_en       TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    -- Una reseña por cuenta y anunciante en cada PH (editarla la reemplaza).
    UNIQUE (tenant_id, anunciante_id, cuenta_id),
    CHECK (estado = 'publicada' OR motivo_moderacion IS NOT NULL)
);
CREATE INDEX ix_resenas_anunciante ON resenas (anunciante_id) WHERE estado = 'publicada';
COMMENT ON COLUMN resenas.texto IS
    'dato_personal: opinión del residente; se muestra sin autor y solo en su copropiedad';

CREATE TABLE reportes_resena (
    tenant_id    UUID        NOT NULL,
    resena_id    UUID        NOT NULL,
    cuenta_id    UUID        NOT NULL REFERENCES cuentas (id),
    motivo       TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 1 AND 500),
    creado_en    TIMESTAMPTZ NOT NULL,
    atendido_en  TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, resena_id, cuenta_id),
    FOREIGN KEY (tenant_id, resena_id) REFERENCES resenas (tenant_id, id)
);
CREATE INDEX ix_reportes_resena_pendientes ON reportes_resena (resena_id)
    WHERE atendido_en IS NULL;
COMMENT ON COLUMN reportes_resena.motivo IS
    'dato_personal: texto libre de quien reporta';

-- ---------------------------------------------------------------------
-- Cobertura (misma regla que domain::directorio::cubre)
-- ---------------------------------------------------------------------
CREATE FUNCTION distancia_km(lat1 NUMERIC, lon1 NUMERIC, lat2 NUMERIC, lon2 NUMERIC)
RETURNS DOUBLE PRECISION
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT 2 * 6371.0 * asin(least(1.0, sqrt(
        power(sin(radians((lat2 - lat1)::float8) / 2), 2)
        + cos(radians(lat1::float8)) * cos(radians(lat2::float8))
          * power(sin(radians((lon2 - lon1)::float8) / 2), 2))))
$$;

CREATE FUNCTION anunciante_cubre(
    p_cobertura TEXT, p_radio_km SMALLINT,
    p_a_lat NUMERIC, p_a_lon NUMERIC, p_a_municipio TEXT,
    p_c_lat NUMERIC, p_c_lon NUMERIC, p_c_municipio TEXT)
RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE p_cobertura
        WHEN 'nacional' THEN TRUE
        WHEN 'municipio' THEN coalesce(p_a_municipio = p_c_municipio, FALSE)
        WHEN 'radio' THEN
            CASE WHEN p_a_lat IS NOT NULL AND p_c_lat IS NOT NULL
                 THEN distancia_km(p_a_lat, p_a_lon, p_c_lat, p_c_lon) <= p_radio_km
                 ELSE coalesce(p_a_municipio = p_c_municipio, FALSE)
            END
        ELSE FALSE
    END
$$;

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER (cruzan ámbitos; devuelven lo mínimo)
-- ---------------------------------------------------------------------
-- Fichas visibles en la copropiedad de la transacción (`app.tenant_id`): anunciante
-- verificado, ficha publicada, cobertura que la alcanza y ninguna categoría restringida.
-- Filtros opcionales: un anunciante, una categoría (o su rama), un texto (nombre,
-- descripción, categorías y sinónimos, sin tildes) y solo destacadas. Página por `id`.
-- El promedio de calificaciones suma las reseñas publicadas de todas las PH; el texto de las
-- reseñas no sale de aquí.
CREATE FUNCTION fichas_directorio_copropiedad(
    p_anunciante UUID, p_categoria TEXT, p_texto TEXT, p_solo_destacadas BOOLEAN,
    p_ahora TIMESTAMPTZ, p_despues UUID, p_limite INTEGER)
RETURNS TABLE (
    anunciante_id UUID, nombre_comercial TEXT, descripcion TEXT, categorias TEXT[],
    telefono TEXT, whatsapp TEXT, correo TEXT, sitio_web TEXT, horario TEXT, direccion TEXT,
    municipio_dane TEXT, destacada BOOLEAN, distancia_km NUMERIC,
    calificacion_promedio NUMERIC, resenas BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH ph AS (
        SELECT c.latitud, c.longitud, c.municipio_dane FROM copropiedades c
        WHERE c.id = app_tenant_id()
    ), q AS (
        SELECT normalizar(nullif(btrim(p_texto), '')) AS texto
    )
    SELECT a.id, a.nombre_comercial, f.descripcion,
           (SELECT array_agg(fc.categoria ORDER BY fc.orden) FROM ficha_categorias fc
            WHERE fc.anunciante_id = a.id),
           f.telefono, f.whatsapp, f.correo, f.sitio_web, f.horario, a.direccion,
           a.municipio_dane,
           coalesce(f.destacada_hasta > p_ahora, FALSE),
           CASE WHEN a.latitud IS NOT NULL AND ph.latitud IS NOT NULL
                THEN round(distancia_km(a.latitud, a.longitud, ph.latitud, ph.longitud)::numeric, 1)
           END,
           r.promedio, coalesce(r.total, 0)
    FROM anunciantes a
    JOIN fichas_directorio f ON f.anunciante_id = a.id
    CROSS JOIN ph
    CROSS JOIN q
    LEFT JOIN LATERAL (
        SELECT round(avg(re.calificacion), 1) AS promedio, count(*) AS total
        FROM resenas re WHERE re.anunciante_id = a.id AND re.estado = 'publicada'
    ) r ON TRUE
    WHERE a.estado_verificacion = 'verificado'
      AND f.publicada
      AND anunciante_cubre(f.cobertura, f.radio_km, a.latitud, a.longitud, a.municipio_dane,
                           ph.latitud, ph.longitud, ph.municipio_dane)
      AND NOT EXISTS (
          SELECT 1 FROM ficha_categorias fc JOIN categorias_publicidad cp ON cp.codigo = fc.categoria
          WHERE fc.anunciante_id = a.id AND cp.restringida)
      AND (p_anunciante IS NULL OR a.id = p_anunciante)
      AND (p_categoria IS NULL OR EXISTS (
          SELECT 1 FROM ficha_categorias fc JOIN categorias_publicidad cp ON cp.codigo = fc.categoria
          WHERE fc.anunciante_id = a.id AND (cp.codigo = p_categoria OR cp.padre = p_categoria)))
      AND (q.texto IS NULL
           OR normalizar(a.nombre_comercial) LIKE '%' || q.texto || '%'
           OR normalizar(coalesce(f.descripcion, '')) LIKE '%' || q.texto || '%'
           OR EXISTS (
               SELECT 1 FROM ficha_categorias fc
               JOIN categorias_publicidad cp ON cp.codigo = fc.categoria
               LEFT JOIN categorias_publicidad pa ON pa.codigo = cp.padre
               WHERE fc.anunciante_id = a.id
                 AND (normalizar(cp.nombre) LIKE '%' || q.texto || '%'
                      OR normalizar(coalesce(pa.nombre, '')) LIKE '%' || q.texto || '%'
                      OR EXISTS (SELECT 1 FROM unnest(cp.sinonimos) s
                                 WHERE normalizar(s) LIKE '%' || q.texto || '%'
                                    OR q.texto LIKE '%' || normalizar(s) || '%'))))
      AND (NOT p_solo_destacadas OR f.destacada_hasta > p_ahora)
      AND (p_despues IS NULL OR a.id > p_despues)
    ORDER BY a.id
    LIMIT p_limite
$$;
REVOKE ALL ON FUNCTION fichas_directorio_copropiedad(UUID, TEXT, TEXT, BOOLEAN, TIMESTAMPTZ, UUID, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fichas_directorio_copropiedad(UUID, TEXT, TEXT, BOOLEAN, TIMESTAMPTZ, UUID, INTEGER) TO dad_app;

-- Cola de moderación de la plataforma: reseñas publicadas con reportes sin atender, de todas
-- las PH. Solo en una transacción de plataforma.
CREATE FUNCTION resenas_reportadas(p_despues UUID, p_limite INTEGER)
RETURNS TABLE (
    resena_id UUID, tenant_id UUID, copropiedad TEXT, anunciante_id UUID,
    nombre_comercial TEXT, calificacion SMALLINT, texto TEXT, reportes BIGINT,
    motivos TEXT[], creada_en TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT re.id, re.tenant_id, c.nombre, re.anunciante_id, a.nombre_comercial,
           re.calificacion, re.texto, count(*), array_agg(rp.motivo ORDER BY rp.creado_en),
           re.creada_en
    FROM resenas re
    JOIN reportes_resena rp ON rp.tenant_id = re.tenant_id AND rp.resena_id = re.id
                           AND rp.atendido_en IS NULL
    JOIN copropiedades c ON c.id = re.tenant_id
    JOIN anunciantes a ON a.id = re.anunciante_id
    WHERE app_plataforma() AND re.estado = 'publicada'
      AND (p_despues IS NULL OR re.id > p_despues)
    GROUP BY re.id, c.nombre, a.nombre_comercial
    ORDER BY re.id
    LIMIT p_limite
$$;
REVOKE ALL ON FUNCTION resenas_reportadas(UUID, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resenas_reportadas(UUID, INTEGER) TO dad_app;

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
-- El catálogo de categorías no tiene datos sensibles: se lee desde cualquier transacción y
-- solo la plataforma lo escribe.
ALTER TABLE categorias_publicidad ENABLE ROW LEVEL SECURITY;
CREATE POLICY categorias_ver ON categorias_publicidad FOR SELECT USING (TRUE);
CREATE POLICY categorias_crear ON categorias_publicidad FOR INSERT WITH CHECK (app_plataforma());
CREATE POLICY categorias_editar ON categorias_publicidad FOR UPDATE
    USING (app_plataforma()) WITH CHECK (app_plataforma());

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['anunciantes', 'fichas_directorio', 'ficha_categorias'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) WITH CHECK (app_plataforma())', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['resenas', 'reportes_resena'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT, UPDATE ON categorias_publicidad, anunciantes, fichas_directorio TO dad_app;
GRANT SELECT, INSERT, DELETE ON ficha_categorias TO dad_app;
GRANT SELECT, INSERT ON resenas, reportes_resena TO dad_app;
GRANT UPDATE (calificacion, texto, actualizada_en, estado, motivo_moderacion, moderada_por,
              moderada_en) ON resenas TO dad_app;
GRANT UPDATE (atendido_en) ON reportes_resena TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('plataforma.directorio.gestionar', 'plataforma'),
    ('directorio.ver', 'copropiedad'),
    ('directorio.resenar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'plataforma', 'plataforma.directorio.gestionar', NULL
FROM unnest(ARRAY['superusuario', 'comercial', 'moderador_publicidad']) AS r;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'directorio.ver', NULL
FROM unnest(ARRAY['administrador', 'asistente_administrativo', 'consejero',
                  'presidente_consejo', 'propietario', 'residente']) AS r;

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'directorio.resenar', NULL
FROM unnest(ARRAY['administrador', 'propietario', 'residente']) AS r;

-- ---------------------------------------------------------------------
-- Catálogo inicial de categorías (la plataforma lo amplía sin desplegar)
-- ---------------------------------------------------------------------
INSERT INTO categorias_publicidad (codigo, padre, nombre, sinonimos, restringida) VALUES
    ('comercio', NULL, 'Comercio', '{}', FALSE),
    ('servicios_tecnicos', NULL, 'Servicios técnicos', '{técnico,reparación,arreglo}', FALSE),
    ('oficios', NULL, 'Oficios', '{}', FALSE),
    ('profesionales', NULL, 'Profesionales', '{asesoría}', FALSE),
    ('servicios_ph', NULL, 'Servicios para propiedad horizontal', '{proveedores para la copropiedad}', FALSE);

INSERT INTO categorias_publicidad (codigo, padre, nombre, sinonimos, restringida) VALUES
    ('tienda', 'comercio', 'Tienda de barrio', '{tienda,minimercado,abarrotes,víveres}', FALSE),
    ('supermercado', 'comercio', 'Supermercado', '{súper,autoservicio,mercado}', FALSE),
    ('farmacia', 'comercio', 'Droguería y farmacia', '{droguería,farmacia,botica}', FALSE),
    ('restaurante', 'comercio', 'Restaurante', '{comida,almuerzo,corrientazo,domicilios de comida}', FALSE),
    ('panaderia', 'comercio', 'Panadería y pastelería', '{pan,pastelería,ponqué}', FALSE),
    ('licores', 'comercio', 'Licores', '{licorera,trago,cerveza}', TRUE),
    ('apuestas', 'comercio', 'Apuestas y juegos de azar', '{chance,lotería,apuestas}', TRUE),
    ('medicamentos_formula', 'comercio', 'Medicamentos con fórmula médica', '{medicamentos de control}', TRUE),
    ('tecnico_televisores', 'servicios_tecnicos', 'Técnico de televisores', '{televisor,tv,arreglo de televisores}', FALSE),
    ('tecnico_lavadoras', 'servicios_tecnicos', 'Técnico de lavadoras', '{lavadora,secadora,arreglo de lavadoras}', FALSE),
    ('tecnico_neveras', 'servicios_tecnicos', 'Técnico de neveras', '{nevera,nevecón,refrigerador}', FALSE),
    ('cerrajeria', 'servicios_tecnicos', 'Cerrajería', '{cerrajero,llaves,chapas}', FALSE),
    ('plomeria', 'servicios_tecnicos', 'Plomería', '{plomero,fontanero,fuga de agua,tubería}', FALSE),
    ('electricista', 'servicios_tecnicos', 'Electricista', '{eléctrico,instalaciones eléctricas}', FALSE),
    ('costura', 'oficios', 'Costura y modistería', '{costurera,modista,modistería,arreglos de ropa,sastre}', FALSE),
    ('belleza_domicilio', 'oficios', 'Belleza a domicilio', '{peluquería,manicure,uñas,barbería}', FALSE),
    ('contadores', 'profesionales', 'Contadores', '{contador,contabilidad,declaración de renta}', FALSE),
    ('abogados', 'profesionales', 'Abogados', '{abogado,asesoría jurídica}', FALSE),
    ('seguridad', 'servicios_ph', 'Seguridad y vigilancia', '{vigilancia,portería}', FALSE),
    ('aseo', 'servicios_ph', 'Aseo y servicios generales', '{limpieza,servicios generales}', FALSE),
    ('jardineria', 'servicios_ph', 'Jardinería', '{jardinero,poda,zonas verdes}', FALSE),
    ('impermeabilizacion', 'servicios_ph', 'Impermeabilización', '{goteras,filtraciones,cubiertas}', FALSE);
