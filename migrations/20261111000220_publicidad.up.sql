-- Publicidad local de la plataforma (plan 08, fases 3 a 6; ADR 0015).
--
-- La publicidad es un negocio de la plataforma: las copropiedades no venden, no cobran ni
-- configuran nada; solo son el canal hacia los residentes que autorizaron la finalidad
-- `publicidad` **frente a la plataforma** (política propia: `politicas_publicidad`).
--
-- Plataforma (RLS `solo_plataforma`): campañas, segmentación por perfil de la PH (nunca de
-- personas), creativos con moderación individual, tarifas, entregas agregadas por
-- (creativo, PH, canal, día) sin identificar a nadie y liquidaciones sin reparto a las PH.
-- Copropiedad (RLS por tenant): envíos masivos (tope de frecuencia), reportes de anuncios y
-- la pantalla del lobby. Lo que cruza ámbitos va por funciones SECURITY DEFINER.

-- ---------------------------------------------------------------------
-- Política de publicidad de la plataforma y autorizaciones del titular
-- ---------------------------------------------------------------------
CREATE TABLE politicas_publicidad (
    id             UUID        PRIMARY KEY,
    version        INTEGER     NOT NULL UNIQUE CHECK (version > 0),
    texto          TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 50000),
    vigente_desde  TIMESTAMPTZ NOT NULL,
    publicada_por  UUID        REFERENCES cuentas (id),
    creada_en      TIMESTAMPTZ NOT NULL
);

-- La autorización de publicidad se da bajo la política de la plataforma, no la de la PH.
ALTER TABLE autorizaciones_tratamiento
    ALTER COLUMN politica_id DROP NOT NULL,
    ADD COLUMN politica_publicidad_id UUID REFERENCES politicas_publicidad (id);
ALTER TABLE autorizaciones_tratamiento ADD CONSTRAINT ck_autorizacion_politica CHECK (
    (finalidad = 'publicidad') = (politica_publicidad_id IS NOT NULL)
    AND (finalidad = 'publicidad' OR politica_id IS NOT NULL)) NOT VALID;

-- ---------------------------------------------------------------------
-- Campañas y creativos (plataforma)
-- ---------------------------------------------------------------------
CREATE TABLE campanas (
    id                  UUID          PRIMARY KEY,
    anunciante_id       UUID          NOT NULL REFERENCES anunciantes (id),
    nombre              TEXT          NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 120),
    objetivo            TEXT          NOT NULL CHECK (objetivo IN ('visibilidad', 'trafico', 'cupones', 'leads')),
    categoria           TEXT          NOT NULL REFERENCES categorias_publicidad (codigo),
    inicio              TIMESTAMPTZ   NOT NULL,
    fin                 TIMESTAMPTZ   NOT NULL CHECK (fin > inicio),
    modelo              TEXT          NOT NULL CHECK (modelo IN ('tarifa_fija', 'cpm', 'cpc',
                            'por_redencion', 'por_lead', 'directorio_destacado')),
    presupuesto_total   NUMERIC(14,2) NOT NULL CHECK (presupuesto_total > 0),
    presupuesto_diario  NUMERIC(14,2) CHECK (presupuesto_diario > 0 AND presupuesto_diario <= presupuesto_total),
    -- Tarifa vigente al aprobar (la liquidación no cambia si la tarifa cambia después).
    tarifa              NUMERIC(14,4) CHECK (tarifa >= 0),
    consumido           NUMERIC(16,4) NOT NULL DEFAULT 0 CHECK (consumido >= 0),
    seg_radio_km        SMALLINT      CHECK (seg_radio_km BETWEEN 1 AND 100),
    seg_municipios      TEXT[]        NOT NULL DEFAULT '{}',
    seg_tipos           TEXT[]        NOT NULL DEFAULT '{}',
    seg_bienes_min      INTEGER       CHECK (seg_bienes_min >= 0),
    seg_bienes_max      INTEGER       CHECK (seg_bienes_max >= seg_bienes_min),
    seg_estratos        SMALLINT[]    NOT NULL DEFAULT '{}',
    -- El anunciante presta servicio a una PH alcanzada (contrato vigente): moderación siempre.
    conflicto_interes   BOOLEAN       NOT NULL DEFAULT FALSE,
    estado              TEXT          NOT NULL CHECK (estado IN ('borrador', 'en_revision', 'aprobada',
                            'rechazada', 'activa', 'pausada', 'finalizada')),
    motivo_estado       TEXT          CHECK (char_length(motivo_estado) BETWEEN 1 AND 500),
    revisada_por        UUID          REFERENCES cuentas (id),
    revisada_en         TIMESTAMPTZ,
    creada_por          UUID          REFERENCES cuentas (id),
    creada_en           TIMESTAMPTZ   NOT NULL,
    actualizada_en      TIMESTAMPTZ   NOT NULL,
    CHECK (estado IN ('borrador', 'en_revision', 'rechazada') OR tarifa IS NOT NULL),
    CHECK (estado <> 'rechazada' OR motivo_estado IS NOT NULL)
);
CREATE INDEX ix_campanas_anunciante ON campanas (anunciante_id, id);
CREATE INDEX ix_campanas_estado ON campanas (estado, fin);
-- Invariante 9 del plan 08: tampoco una campaña usa una categoría restringida.
CREATE TRIGGER campana_sin_categoria_restringida
    BEFORE INSERT OR UPDATE OF categoria ON campanas
    FOR EACH ROW EXECUTE FUNCTION rechazar_categoria_restringida();

CREATE TABLE campana_copropiedades (
    campana_id  UUID NOT NULL REFERENCES campanas (id) ON DELETE CASCADE,
    tenant_id   UUID NOT NULL REFERENCES copropiedades (id),
    PRIMARY KEY (campana_id, tenant_id)
);

CREATE TABLE creativos (
    id                  UUID        PRIMARY KEY,
    campana_id          UUID        NOT NULL REFERENCES campanas (id),
    anunciante_id       UUID        NOT NULL REFERENCES anunciantes (id),
    formato             TEXT        NOT NULL CHECK (formato IN ('banner', 'tarjeta_destacada',
                            'oferta_cupon', 'mensaje_whatsapp', 'boletin_correo', 'pantalla_lobby')),
    titulo              TEXT        NOT NULL CHECK (char_length(titulo) BETWEEN 1 AND 60),
    texto               TEXT        NOT NULL CHECK (char_length(texto) BETWEEN 1 AND 500),
    boton               TEXT        CHECK (char_length(boton) BETWEEN 1 AND 20),
    enlace              TEXT        CHECK (enlace ~ '^https://' AND char_length(enlace) <= 300),
    condiciones         TEXT        CHECK (char_length(condiciones) BETWEEN 1 AND 1000),
    es_promocion        BOOLEAN     NOT NULL,
    cupon_codigo        TEXT        CHECK (cupon_codigo ~ '^[A-Z0-9]{4,20}$'),
    cupon_limite        INTEGER     CHECK (cupon_limite > 0),
    cupon_usos          INTEGER     NOT NULL DEFAULT 0 CHECK (cupon_usos >= 0),
    plantilla_whatsapp  TEXT        CHECK (plantilla_whatsapp ~ '^[a-z0-9_]+$' AND char_length(plantilla_whatsapp) <= 512),
    estado              TEXT        NOT NULL CHECK (estado IN ('pendiente', 'aprobado', 'rechazado')),
    motivo_estado       TEXT        CHECK (char_length(motivo_estado) BETWEEN 1 AND 500),
    creado_en           TIMESTAMPTZ NOT NULL,
    actualizado_en      TIMESTAMPTZ NOT NULL,
    CHECK ((formato = 'oferta_cupon') = (cupon_codigo IS NOT NULL)),
    CHECK ((formato = 'mensaje_whatsapp') = (plantilla_whatsapp IS NOT NULL)),
    -- Ley 1480: una promoción declara sus condiciones.
    CHECK (NOT es_promocion OR condiciones IS NOT NULL),
    CHECK (cupon_limite IS NULL OR cupon_usos <= cupon_limite),
    CHECK (estado <> 'rechazado' OR motivo_estado IS NOT NULL)
);
CREATE INDEX ix_creativos_campana ON creativos (campana_id);
CREATE INDEX ix_creativos_pendientes ON creativos (id) WHERE estado = 'pendiente';
CREATE UNIQUE INDEX ux_creativos_cupon ON creativos (anunciante_id, cupon_codigo)
    WHERE cupon_codigo IS NOT NULL;

-- ---------------------------------------------------------------------
-- Tarifas, entregas y liquidaciones (plataforma)
-- ---------------------------------------------------------------------
CREATE TABLE tarifas_publicidad (
    id             UUID          PRIMARY KEY,
    modelo         TEXT          NOT NULL CHECK (modelo IN ('tarifa_fija', 'cpm', 'cpc',
                       'por_redencion', 'por_lead', 'directorio_destacado')),
    valor          NUMERIC(14,4) NOT NULL CHECK (valor >= 0),
    vigente_desde  DATE          NOT NULL,
    creada_por     UUID          REFERENCES cuentas (id),
    creada_en      TIMESTAMPTZ   NOT NULL,
    UNIQUE (modelo, vigente_desde)
);

-- Agregado por (creativo, PH, canal, día): nunca identifica al residente. `tenant_id` NULL =
-- redención en el mostrador del comercio.
CREATE TABLE entregas_publicidad (
    creativo_id  UUID          NOT NULL REFERENCES creativos (id),
    campana_id   UUID          NOT NULL REFERENCES campanas (id),
    tenant_id    UUID          REFERENCES copropiedades (id),
    canal        TEXT          NOT NULL CHECK (canal IN ('app', 'pantalla', 'whatsapp', 'correo')),
    dia          DATE          NOT NULL,
    impresiones  INTEGER       NOT NULL DEFAULT 0 CHECK (impresiones >= 0),
    clics        INTEGER       NOT NULL DEFAULT 0 CHECK (clics >= 0),
    redenciones  INTEGER       NOT NULL DEFAULT 0 CHECK (redenciones >= 0),
    costo        NUMERIC(16,4) NOT NULL DEFAULT 0 CHECK (costo >= 0),
    UNIQUE NULLS NOT DISTINCT (creativo_id, tenant_id, canal, dia)
);
CREATE INDEX ix_entregas_campana_dia ON entregas_publicidad (campana_id, dia);

CREATE TABLE liquidaciones (
    id              UUID          PRIMARY KEY,
    anunciante_id   UUID          NOT NULL REFERENCES anunciantes (id),
    periodo         DATE          NOT NULL CHECK (extract(day FROM periodo) = 1),
    variable        NUMERIC(16,2) NOT NULL CHECK (variable >= 0),
    fijos           NUMERIC(16,2) NOT NULL CHECK (fijos >= 0),
    leads           INTEGER       NOT NULL CHECK (leads >= 0),
    valor_leads     NUMERIC(16,2) NOT NULL CHECK (valor_leads >= 0),
    valor           NUMERIC(16,2) NOT NULL CHECK (valor >= 0),
    -- Lo que le costó a la plataforma enviar sus mensajes de WhatsApp (tarifas_whatsapp).
    costo_whatsapp  NUMERIC(16,2) NOT NULL CHECK (costo_whatsapp >= 0),
    estado          TEXT          NOT NULL CHECK (estado IN ('calculada', 'facturada', 'pagada')),
    calculada_en    TIMESTAMPTZ   NOT NULL,
    actualizada_en  TIMESTAMPTZ   NOT NULL,
    UNIQUE (anunciante_id, periodo)
);

-- ---------------------------------------------------------------------
-- Por copropiedad: envíos masivos, reportes y pantalla del lobby
-- ---------------------------------------------------------------------
CREATE TABLE envios_publicidad (
    tenant_id   UUID        NOT NULL REFERENCES copropiedades (id),
    campana_id  UUID        NOT NULL REFERENCES campanas (id),
    persona_id  UUID        NOT NULL,
    canal       TEXT        NOT NULL CHECK (canal IN ('whatsapp', 'correo')),
    enviado_en  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, campana_id, persona_id, canal),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id)
);
CREATE INDEX ix_envios_publicidad_frecuencia
    ON envios_publicidad (tenant_id, persona_id, canal, enviado_en DESC);

CREATE TABLE reportes_publicidad (
    id           UUID        PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES copropiedades (id),
    creativo_id  UUID        NOT NULL REFERENCES creativos (id),
    cuenta_id    UUID        NOT NULL REFERENCES cuentas (id),
    motivo       TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 1 AND 500),
    creado_en    TIMESTAMPTZ NOT NULL,
    atendido_en  TIMESTAMPTZ,
    UNIQUE (tenant_id, creativo_id, cuenta_id)
);
CREATE INDEX ix_reportes_publicidad_pendientes ON reportes_publicidad (creativo_id)
    WHERE atendido_en IS NULL;
COMMENT ON COLUMN reportes_publicidad.motivo IS 'dato_personal: texto libre de quien reporta';

CREATE TABLE pantallas_publicidad (
    tenant_id   UUID        PRIMARY KEY REFERENCES copropiedades (id),
    token_hash  BYTEA       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    creado_por  UUID        REFERENCES cuentas (id),
    creado_en   TIMESTAMPTZ NOT NULL
);

-- El costo de WhatsApp de la pauta se lleva por campaña.
ALTER TABLE outbox ADD COLUMN campana_id UUID;
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check
    CHECK (concepto IN ('control_acceso', 'comunicados', 'directorio', 'publicidad'));
CREATE INDEX ix_outbox_campana ON outbox (campana_id, creado_en) WHERE campana_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Alcance (misma regla que domain::publicidad::Segmentacion::alcanza)
-- ---------------------------------------------------------------------
CREATE FUNCTION campana_alcanza(p_campana UUID, p_tenant UUID)
RETURNS BOOLEAN
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    SELECT coalesce((
        SELECT (NOT EXISTS (SELECT 1 FROM campana_copropiedades x WHERE x.campana_id = c.id)
                OR EXISTS (SELECT 1 FROM campana_copropiedades x
                           WHERE x.campana_id = c.id AND x.tenant_id = p.id))
           AND (c.seg_radio_km IS NULL
                OR anunciante_cubre('radio', c.seg_radio_km, a.latitud, a.longitud,
                                    a.municipio_dane, p.latitud, p.longitud, p.municipio_dane))
           AND (cardinality(c.seg_municipios) = 0 OR p.municipio_dane = ANY (c.seg_municipios))
           AND (cardinality(c.seg_tipos) = 0 OR p.tipo = ANY (c.seg_tipos))
           AND (c.seg_bienes_min IS NULL OR b.n >= c.seg_bienes_min)
           AND (c.seg_bienes_max IS NULL OR b.n <= c.seg_bienes_max)
           AND (cardinality(c.seg_estratos) = 0 OR p.estrato = ANY (c.seg_estratos))
        FROM campanas c
        JOIN anunciantes a ON a.id = c.anunciante_id
        CROSS JOIN copropiedades p
        CROSS JOIN LATERAL (
            SELECT count(*) AS n FROM bienes_privados bp WHERE bp.tenant_id = p.id AND bp.activo
        ) b
        WHERE c.id = p_campana AND p.id = p_tenant), FALSE)
$$;
REVOKE ALL ON FUNCTION campana_alcanza(UUID, UUID) FROM PUBLIC;

-- Día de los topes diarios (hora de Colombia).
CREATE FUNCTION dia_publicidad(p_ahora TIMESTAMPTZ) RETURNS DATE
LANGUAGE sql IMMUTABLE AS $$ SELECT (p_ahora AT TIME ZONE 'America/Bogota')::date $$;

-- ¿La copropiedad de la transacción tiene el módulo `publicidad` habilitado? (Invariante 3.)
CREATE FUNCTION publicidad_habilitada(p_tenant UUID, p_ahora TIMESTAMPTZ) RETURNS BOOLEAN
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    SELECT EXISTS (SELECT 1 FROM modulos_habilitados m
                   WHERE m.tenant_id = p_tenant AND m.modulo = 'publicidad'
                     AND m.desde <= p_ahora AND (m.hasta IS NULL OR m.hasta > p_ahora))
$$;
REVOKE ALL ON FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ) FROM PUBLIC;

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER para la copropiedad de la transacción
-- ---------------------------------------------------------------------
-- Piezas que pueden entregarse en la PH (`app.tenant_id`): módulo habilitado, anunciante
-- verificado, creativo aprobado, campaña activa y vigente que la alcanza. El dominio vuelve a
-- verificar todo y decide con el presupuesto (`PiezaCandidata::entregable`).
CREATE FUNCTION piezas_publicidad(p_formatos TEXT[], p_ahora TIMESTAMPTZ)
RETURNS TABLE (
    creativo_id UUID, campana_id UUID, anunciante_id UUID, nombre_comercial TEXT,
    anunciante_verificado BOOLEAN, formato TEXT, titulo TEXT, texto TEXT, boton TEXT,
    enlace TEXT, condiciones TEXT, es_promocion BOOLEAN, cupon_codigo TEXT,
    plantilla_whatsapp TEXT, estado_creativo TEXT, estado_campana TEXT, inicio TIMESTAMPTZ,
    fin TIMESTAMPTZ, modelo TEXT, tarifa NUMERIC, presupuesto_total NUMERIC,
    presupuesto_diario NUMERIC, consumido NUMERIC, consumido_hoy NUMERIC,
    impresiones_hoy BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT cr.id, c.id, a.id, a.nombre_comercial, a.estado_verificacion = 'verificado',
           cr.formato, cr.titulo, cr.texto, cr.boton, cr.enlace, cr.condiciones,
           cr.es_promocion, cr.cupon_codigo, cr.plantilla_whatsapp, cr.estado, c.estado,
           c.inicio, c.fin, c.modelo, c.tarifa, c.presupuesto_total, c.presupuesto_diario,
           c.consumido,
           (SELECT coalesce(sum(e.costo), 0) FROM entregas_publicidad e
            WHERE e.campana_id = c.id AND e.dia = dia_publicidad(p_ahora)),
           (SELECT coalesce(sum(e.impresiones), 0) FROM entregas_publicidad e
            WHERE e.creativo_id = cr.id AND e.tenant_id = app_tenant_id()
              AND e.dia = dia_publicidad(p_ahora))
    FROM creativos cr
    JOIN campanas c ON c.id = cr.campana_id
    JOIN anunciantes a ON a.id = c.anunciante_id
    WHERE publicidad_habilitada(app_tenant_id(), p_ahora)
      AND a.estado_verificacion = 'verificado'
      AND cr.estado = 'aprobado'
      AND c.estado = 'activa' AND c.inicio <= p_ahora AND p_ahora < c.fin
      AND cr.formato = ANY (p_formatos)
      AND campana_alcanza(c.id, app_tenant_id())
    ORDER BY cr.id
$$;

-- La misma fila de una pieza, con la campaña bloqueada (`FOR UPDATE`) para registrar una
-- entrega sin superar los topes aunque haya entregas concurrentes (invariante 6). Vacío si la
-- pieza no se puede entregar en la PH.
CREATE FUNCTION pieza_para_entrega(p_creativo UUID, p_ahora TIMESTAMPTZ)
RETURNS TABLE (
    creativo_id UUID, campana_id UUID, anunciante_verificado BOOLEAN, formato TEXT,
    estado_creativo TEXT, estado_campana TEXT, inicio TIMESTAMPTZ, fin TIMESTAMPTZ,
    modelo TEXT, tarifa NUMERIC, presupuesto_total NUMERIC, presupuesto_diario NUMERIC,
    consumido NUMERIC, consumido_hoy NUMERIC)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_campana UUID;
BEGIN
    SELECT cr.campana_id INTO v_campana FROM creativos cr WHERE cr.id = p_creativo;
    IF v_campana IS NULL OR NOT publicidad_habilitada(app_tenant_id(), p_ahora)
       OR NOT campana_alcanza(v_campana, app_tenant_id()) THEN
        RETURN;
    END IF;
    PERFORM 1 FROM campanas c WHERE c.id = v_campana FOR UPDATE;
    RETURN QUERY
    SELECT cr.id, c.id, a.estado_verificacion = 'verificado', cr.formato, cr.estado, c.estado,
           c.inicio, c.fin, c.modelo, c.tarifa, c.presupuesto_total, c.presupuesto_diario,
           c.consumido,
           (SELECT coalesce(sum(e.costo), 0) FROM entregas_publicidad e
            WHERE e.campana_id = c.id AND e.dia = dia_publicidad(p_ahora))
    FROM creativos cr
    JOIN campanas c ON c.id = cr.campana_id
    JOIN anunciantes a ON a.id = c.anunciante_id
    WHERE cr.id = p_creativo;
END
$$;

-- Suma una entrega agregada (sin personas) y su costo; `p_finalizar` cierra la campaña
-- (presupuesto agotado). La PH es la de la transacción; en la plataforma (redención en el
-- mostrador) queda NULL. El llamador ya bloqueó la campaña.
CREATE FUNCTION aplicar_entrega_publicidad(
    p_creativo UUID, p_canal TEXT, p_ahora TIMESTAMPTZ, p_impresiones INTEGER,
    p_clics INTEGER, p_redenciones INTEGER, p_costo NUMERIC, p_finalizar BOOLEAN)
RETURNS VOID
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_campana UUID;
    v_tenant UUID := CASE WHEN app_plataforma() THEN NULL ELSE app_tenant_id() END;
BEGIN
    SELECT cr.campana_id INTO STRICT v_campana FROM creativos cr WHERE cr.id = p_creativo;
    IF v_tenant IS NOT NULL AND NOT campana_alcanza(v_campana, v_tenant) THEN
        RAISE EXCEPTION 'la campaña no alcanza a la copropiedad' USING ERRCODE = 'check_violation';
    END IF;
    INSERT INTO entregas_publicidad AS e (creativo_id, campana_id, tenant_id, canal, dia,
                                          impresiones, clics, redenciones, costo)
    VALUES (p_creativo, v_campana, v_tenant, p_canal, dia_publicidad(p_ahora), p_impresiones,
            p_clics, p_redenciones, p_costo)
    ON CONFLICT (creativo_id, tenant_id, canal, dia) DO UPDATE
        SET impresiones = e.impresiones + EXCLUDED.impresiones,
            clics = e.clics + EXCLUDED.clics,
            redenciones = e.redenciones + EXCLUDED.redenciones,
            costo = e.costo + EXCLUDED.costo;
    UPDATE campanas SET consumido = consumido + p_costo,
                        estado = CASE WHEN p_finalizar THEN 'finalizada' ELSE estado END,
                        actualizada_en = p_ahora
    WHERE id = v_campana;
END
$$;

-- Copropiedades activas con el módulo `publicidad` (worker de envíos masivos).
CREATE FUNCTION copropiedades_con_publicidad(p_ahora TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT c.id FROM copropiedades c
    WHERE c.estado = 'activa' AND publicidad_habilitada(c.id, p_ahora)
    ORDER BY c.id
$$;

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER para la plataforma (exigen `app.plataforma`)
-- ---------------------------------------------------------------------
-- Conflicto de interés: la organización del anunciante tiene un contrato de servicio vigente
-- en alguna copropiedad que la campaña alcanza.
CREATE FUNCTION conflicto_interes_campana(p_campana UUID)
RETURNS BOOLEAN
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT app_plataforma() AND EXISTS (
        SELECT 1 FROM campanas c
        JOIN anunciantes a ON a.id = c.anunciante_id
        JOIN contratos_servicio cs ON cs.organizacion_id = a.organizacion_id
        WHERE c.id = p_campana AND cs.estado = 'vigente' AND cs.vigente_desde <= now()
          AND (cs.vigente_hasta IS NULL OR cs.vigente_hasta > now())
          AND campana_alcanza(c.id, cs.tenant_id))
$$;

-- Copropiedades que la campaña alcanza hoy (cuántas, para el anunciante: sin nombres).
CREATE FUNCTION alcance_campana(p_campana UUID)
RETURNS BIGINT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT count(*) FROM copropiedades p
    WHERE app_plataforma() AND p.estado = 'activa' AND publicidad_habilitada(p.id, now())
      AND campana_alcanza(p_campana, p.id)
$$;

-- Contactos autorizados (leads) de un anunciante, el más reciente primero. Solo lo que el
-- residente autorizó compartir para esa solicitud: nombre y celular.
CREATE FUNCTION leads_anunciante(p_anunciante UUID, p_antes UUID, p_limite INTEGER)
RETURNS TABLE (solicitud_id UUID, creado_en TIMESTAMPTZ, categoria TEXT, nombre TEXT,
               telefono TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT s.id, cp.creado_en, s.categoria, pe.nombre, pe.telefono
    FROM contactos_proveedor cp
    JOIN solicitudes_proveedor s ON s.tenant_id = cp.tenant_id AND s.id = cp.solicitud_id
    JOIN personas pe ON pe.tenant_id = s.tenant_id AND pe.id = s.persona_id
    WHERE app_plataforma() AND cp.anunciante_id = p_anunciante AND s.autorizado_en IS NOT NULL
      AND (p_antes IS NULL OR s.id < p_antes)
    ORDER BY s.id DESC
    LIMIT p_limite
$$;

-- Contactos y mensajes de WhatsApp de publicidad por anunciante en un periodo (liquidación).
CREATE FUNCTION consumo_directo_anunciantes(p_desde TIMESTAMPTZ, p_hasta TIMESTAMPTZ)
RETURNS TABLE (anunciante_id UUID, leads BIGINT, mensajes_whatsapp BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH l AS (
        SELECT cp.anunciante_id, count(*) AS n FROM contactos_proveedor cp
        WHERE cp.creado_en >= p_desde AND cp.creado_en < p_hasta GROUP BY 1
    ), m AS (
        SELECT c.anunciante_id, count(*) AS n FROM outbox o
        JOIN campanas c ON c.id = o.campana_id
        WHERE o.concepto = 'publicidad' AND o.canal = 'whatsapp' AND o.estado = 'enviado'
          AND o.creado_en >= p_desde AND o.creado_en < p_hasta GROUP BY 1
    )
    SELECT coalesce(l.anunciante_id, m.anunciante_id), coalesce(l.n, 0), coalesce(m.n, 0)
    FROM l FULL JOIN m ON m.anunciante_id = l.anunciante_id
    WHERE app_plataforma()
$$;

-- Cola de moderación: creativos con reportes sin atender, de todas las PH.
CREATE FUNCTION reportes_publicidad_pendientes(p_despues UUID, p_limite INTEGER)
RETURNS TABLE (creativo_id UUID, campana_id UUID, anunciante_id UUID, nombre_comercial TEXT,
               titulo TEXT, reportes BIGINT, motivos TEXT[], primero_en TIMESTAMPTZ)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT cr.id, cr.campana_id, cr.anunciante_id, a.nombre_comercial, cr.titulo, count(*),
           array_agg(r.motivo ORDER BY r.creado_en), min(r.creado_en)
    FROM reportes_publicidad r
    JOIN creativos cr ON cr.id = r.creativo_id
    JOIN anunciantes a ON a.id = cr.anunciante_id
    WHERE app_plataforma() AND r.atendido_en IS NULL
      AND (p_despues IS NULL OR cr.id > p_despues)
    GROUP BY cr.id, a.nombre_comercial
    ORDER BY cr.id
    LIMIT p_limite
$$;

CREATE FUNCTION atender_reportes_publicidad(p_creativo UUID, p_ahora TIMESTAMPTZ)
RETURNS BIGINT
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH x AS (
        UPDATE reportes_publicidad SET atendido_en = p_ahora
        WHERE app_plataforma() AND creativo_id = p_creativo AND atendido_en IS NULL
        RETURNING 1
    )
    SELECT count(*) FROM x
$$;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'piezas_publicidad(TEXT[], TIMESTAMPTZ)',
        'pieza_para_entrega(UUID, TIMESTAMPTZ)',
        'aplicar_entrega_publicidad(UUID, TEXT, TIMESTAMPTZ, INTEGER, INTEGER, INTEGER, NUMERIC, BOOLEAN)',
        'copropiedades_con_publicidad(TIMESTAMPTZ)',
        'conflicto_interes_campana(UUID)',
        'alcance_campana(UUID)',
        'leads_anunciante(UUID, UUID, INTEGER)',
        'consumo_directo_anunciantes(TIMESTAMPTZ, TIMESTAMPTZ)',
        'reportes_publicidad_pendientes(UUID, INTEGER)',
        'atender_reportes_publicidad(UUID, TIMESTAMPTZ)'
    ] LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f);
        EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO dad_app', f);
    END LOOP;
END
$$;

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
ALTER TABLE politicas_publicidad ENABLE ROW LEVEL SECURITY;
CREATE POLICY politicas_publicidad_ver ON politicas_publicidad FOR SELECT USING (TRUE);
CREATE POLICY politicas_publicidad_crear ON politicas_publicidad FOR INSERT
    WITH CHECK (app_plataforma());

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['campanas', 'campana_copropiedades', 'creativos',
                             'tarifas_publicidad', 'entregas_publicidad', 'liquidaciones'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) WITH CHECK (app_plataforma())', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['envios_publicidad', 'reportes_publicidad', 'pantallas_publicidad'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT ON politicas_publicidad TO dad_app;
GRANT SELECT, INSERT, UPDATE ON campanas, creativos, liquidaciones TO dad_app;
GRANT SELECT, INSERT, DELETE ON campana_copropiedades TO dad_app;
GRANT SELECT, INSERT ON tarifas_publicidad TO dad_app;
-- Las entregas solo cambian por `aplicar_entrega_publicidad`.
GRANT SELECT ON entregas_publicidad TO dad_app;
GRANT SELECT, INSERT ON envios_publicidad, reportes_publicidad TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON pantallas_publicidad TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES ('plataforma.publicidad.liquidar', 'plataforma');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'plataforma', 'plataforma.publicidad.liquidar', NULL
FROM unnest(ARRAY['superusuario', 'comercial', 'facturacion']) AS r;
