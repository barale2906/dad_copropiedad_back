-- Plan 20, fase 3 (ADR 0023): la publicidad la opera solo la plataforma.
--
-- 1. Deja de ser un módulo contratable: llega a toda entidad activa que la plataforma no
--    excluya (`exclusiones_publicidad`). `publicidad_habilitada` conserva su firma y ahora mira
--    la exclusión, así todas las funciones que la usan siguen igual.
-- 2. Cuatro ojos: quien carga una campaña o una pieza no la modera.
-- 3. Cupones: el comercio los redime con su enlace propio (sin cuenta).
-- 4. Enlaces estables: el token del directorio público y el de la pantalla se guardan también
--    cifrados (Cofre) para volver a entregarlos sin cambiarlos.
-- 5. Empresas: las piezas, sus eventos y reportes y la segmentación sirven a cualquier entidad;
--    los criterios propios de la PH solo los cumple una PH. Lo masivo y la pantalla siguen
--    siendo de la PH.

-- ---------------------------------------------------------------------
-- 1. Sin módulo; exclusión por entidad (solo la plataforma)
-- ---------------------------------------------------------------------
DELETE FROM modulos_habilitados WHERE modulo = 'publicidad';
ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'facturacion_electronica', 'bienes_comunes',
    'control_acceso', 'asambleas', 'gestion_administrativa'));

CREATE TABLE exclusiones_publicidad (
    tenant_id   UUID        PRIMARY KEY REFERENCES entidades (id),
    motivo      TEXT        NOT NULL CHECK (char_length(motivo) BETWEEN 1 AND 500),
    creada_por  UUID        REFERENCES cuentas (id),
    creada_en   TIMESTAMPTZ NOT NULL
);
ALTER TABLE exclusiones_publicidad ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON exclusiones_publicidad
    USING (app_plataforma()) WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE, DELETE ON exclusiones_publicidad TO dad_app;

-- ¿Hay pauta en la entidad? Toda entidad, salvo la plataforma y las excluidas. SECURITY DEFINER:
-- la exclusión es de plataforma y se consulta desde la transacción de la entidad.
CREATE OR REPLACE FUNCTION publicidad_habilitada(p_tenant UUID, p_ahora TIMESTAMPTZ)
RETURNS BOOLEAN
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT EXISTS (SELECT 1 FROM entidades e WHERE e.id = p_tenant AND NOT e.es_plataforma)
       AND NOT EXISTS (SELECT 1 FROM exclusiones_publicidad x WHERE x.tenant_id = p_tenant)
$$;
REVOKE ALL ON FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ) TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES ('plataforma.publicidad.gestionar', 'plataforma');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT r, 'plataforma', 'plataforma.publicidad.gestionar', NULL
FROM unnest(ARRAY[uuid_rol_plantilla('superusuario'), uuid_rol_plantilla('comercial')]) AS r;

-- ---------------------------------------------------------------------
-- 2. Cuatro ojos en la moderación
-- ---------------------------------------------------------------------
ALTER TABLE campanas ADD COLUMN editada_por UUID REFERENCES cuentas (id);
ALTER TABLE campanas ADD CONSTRAINT ck_campanas_cuatro_ojos CHECK (
    revisada_por IS NULL
    OR (revisada_por IS DISTINCT FROM creada_por AND revisada_por IS DISTINCT FROM editada_por)
) NOT VALID;
ALTER TABLE creativos
    ADD COLUMN cargado_por UUID REFERENCES cuentas (id),
    ADD COLUMN revisado_por UUID REFERENCES cuentas (id);
ALTER TABLE creativos ADD CONSTRAINT ck_creativos_cuatro_ojos
    CHECK (revisado_por IS NULL OR revisado_por IS DISTINCT FROM cargado_por);

-- ---------------------------------------------------------------------
-- 3. Enlace de redención de cupones del comercio (tabla de plataforma)
-- ---------------------------------------------------------------------
CREATE TABLE enlaces_comercio (
    anunciante_id  UUID        PRIMARY KEY REFERENCES anunciantes (id),
    -- SHA-256 del token: con él se reconoce la petición.
    token_hash     BYTEA       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    -- El mismo token cifrado con el Cofre: se vuelve a entregar sin cambiarlo.
    token_cifrado  BYTEA       NOT NULL,
    creado_por     UUID        REFERENCES cuentas (id),
    creado_en      TIMESTAMPTZ NOT NULL
);
ALTER TABLE enlaces_comercio ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON enlaces_comercio
    USING (app_plataforma()) WITH CHECK (app_plataforma());
GRANT SELECT, INSERT, UPDATE, DELETE ON enlaces_comercio TO dad_app;

-- ---------------------------------------------------------------------
-- 4. Enlaces estables
-- ---------------------------------------------------------------------
ALTER TABLE enlaces_directorio ADD COLUMN token_cifrado BYTEA;
ALTER TABLE pantallas_publicidad ADD COLUMN token_cifrado BYTEA;

-- ---------------------------------------------------------------------
-- 5. Publicidad en las empresas
-- ---------------------------------------------------------------------
ALTER TABLE campana_copropiedades DROP CONSTRAINT campana_copropiedades_tenant_id_fkey;
ALTER TABLE campana_copropiedades ADD CONSTRAINT campana_copropiedades_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES entidades (id);
COMMENT ON TABLE campana_copropiedades IS 'Entidades (PH o empresas) elegidas una por una en la segmentación';
ALTER TABLE entregas_publicidad DROP CONSTRAINT entregas_publicidad_tenant_id_fkey;
ALTER TABLE entregas_publicidad ADD CONSTRAINT entregas_publicidad_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES entidades (id);
ALTER TABLE reportes_publicidad DROP CONSTRAINT reportes_publicidad_tenant_id_fkey;
ALTER TABLE reportes_publicidad ADD CONSTRAINT reportes_publicidad_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES entidades (id);

ALTER TABLE campanas ADD COLUMN seg_tipos_entidad TEXT[] NOT NULL DEFAULT '{}'
    CHECK (seg_tipos_entidad <@ ARRAY['propiedad_horizontal', 'empresa']);

-- Misma regla que domain::publicidad::Segmentacion::alcanza.
CREATE OR REPLACE FUNCTION campana_alcanza(p_campana UUID, p_tenant UUID)
RETURNS BOOLEAN
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    SELECT coalesce((
        SELECT (NOT EXISTS (SELECT 1 FROM campana_copropiedades x WHERE x.campana_id = c.id)
                OR EXISTS (SELECT 1 FROM campana_copropiedades x
                           WHERE x.campana_id = c.id AND x.tenant_id = p.id))
           AND (cardinality(c.seg_tipos_entidad) = 0
                OR (CASE WHEN cp.id IS NULL THEN 'empresa' ELSE 'propiedad_horizontal' END)
                   = ANY (c.seg_tipos_entidad))
           AND (c.seg_radio_km IS NULL
                OR anunciante_cubre('radio', c.seg_radio_km, a.latitud, a.longitud,
                                    a.municipio_dane, p.latitud, p.longitud, p.municipio_dane))
           AND (cardinality(c.seg_municipios) = 0 OR p.municipio_dane = ANY (c.seg_municipios))
           -- Criterios propios de la PH: una empresa no los cumple.
           AND (cardinality(c.seg_tipos) = 0 OR coalesce(cp.tipo = ANY (c.seg_tipos), FALSE))
           AND (c.seg_bienes_min IS NULL OR (cp.id IS NOT NULL AND b.n >= c.seg_bienes_min))
           AND (c.seg_bienes_max IS NULL OR (cp.id IS NOT NULL AND b.n <= c.seg_bienes_max))
           AND (cardinality(c.seg_estratos) = 0
                OR coalesce(cp.estrato = ANY (c.seg_estratos), FALSE))
           AND NOT p.es_plataforma
        FROM campanas c
        JOIN anunciantes a ON a.id = c.anunciante_id
        CROSS JOIN entidades p
        LEFT JOIN copropiedades cp ON cp.id = p.id
        CROSS JOIN LATERAL (
            SELECT count(*) AS n FROM bienes_privados bp WHERE bp.tenant_id = p.id AND bp.activo
        ) b
        WHERE c.id = p_campana AND p.id = p_tenant), FALSE)
$$;

-- Los envíos masivos siguen siendo solo de la PH.
CREATE OR REPLACE FUNCTION copropiedades_con_publicidad(p_ahora TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT c.id FROM entidades c
    JOIN copropiedades cp ON cp.id = c.id
    WHERE c.estado = 'activa' AND publicidad_habilitada(c.id, p_ahora)
    ORDER BY c.id
$$;
