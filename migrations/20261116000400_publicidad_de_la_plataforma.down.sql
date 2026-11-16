CREATE OR REPLACE FUNCTION copropiedades_con_publicidad(p_ahora TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT c.id FROM entidades c
    WHERE c.estado = 'activa' AND publicidad_habilitada(c.id, p_ahora)
    ORDER BY c.id
$$;

CREATE OR REPLACE FUNCTION campana_alcanza(p_campana UUID, p_tenant UUID)
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
           AND (cardinality(c.seg_tipos) = 0 OR cp.tipo = ANY (c.seg_tipos))
           AND (c.seg_bienes_min IS NULL OR b.n >= c.seg_bienes_min)
           AND (c.seg_bienes_max IS NULL OR b.n <= c.seg_bienes_max)
           AND (cardinality(c.seg_estratos) = 0 OR cp.estrato = ANY (c.seg_estratos))
        FROM campanas c
        JOIN anunciantes a ON a.id = c.anunciante_id
        CROSS JOIN entidades p
        JOIN copropiedades cp ON cp.id = p.id
        CROSS JOIN LATERAL (
            SELECT count(*) AS n FROM bienes_privados bp WHERE bp.tenant_id = p.id AND bp.activo
        ) b
        WHERE c.id = p_campana AND p.id = p_tenant), FALSE)
$$;
ALTER TABLE campanas DROP COLUMN seg_tipos_entidad;

DELETE FROM reportes_publicidad r WHERE NOT EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = r.tenant_id);
ALTER TABLE reportes_publicidad DROP CONSTRAINT reportes_publicidad_tenant_id_fkey;
ALTER TABLE reportes_publicidad ADD CONSTRAINT reportes_publicidad_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES copropiedades (id);
UPDATE entregas_publicidad e SET tenant_id = NULL
WHERE tenant_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = e.tenant_id);
ALTER TABLE entregas_publicidad DROP CONSTRAINT entregas_publicidad_tenant_id_fkey;
ALTER TABLE entregas_publicidad ADD CONSTRAINT entregas_publicidad_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES copropiedades (id);
DELETE FROM campana_copropiedades x WHERE NOT EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = x.tenant_id);
COMMENT ON TABLE campana_copropiedades IS NULL;
ALTER TABLE campana_copropiedades DROP CONSTRAINT campana_copropiedades_tenant_id_fkey;
ALTER TABLE campana_copropiedades ADD CONSTRAINT campana_copropiedades_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES copropiedades (id);

ALTER TABLE pantallas_publicidad DROP COLUMN token_cifrado;
ALTER TABLE enlaces_directorio DROP COLUMN token_cifrado;
DROP TABLE enlaces_comercio;

ALTER TABLE creativos DROP CONSTRAINT ck_creativos_cuatro_ojos;
ALTER TABLE creativos DROP COLUMN revisado_por, DROP COLUMN cargado_por;
ALTER TABLE campanas DROP CONSTRAINT ck_campanas_cuatro_ojos;
ALTER TABLE campanas DROP COLUMN editada_por;

DELETE FROM roles_permisos WHERE permiso = 'plataforma.publicidad.gestionar';
DELETE FROM permisos WHERE codigo = 'plataforma.publicidad.gestionar';

CREATE OR REPLACE FUNCTION publicidad_habilitada(p_tenant UUID, p_ahora TIMESTAMPTZ)
RETURNS BOOLEAN
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    SELECT EXISTS (SELECT 1 FROM modulos_habilitados m
                   WHERE m.tenant_id = p_tenant AND m.modulo = 'publicidad'
                     AND m.desde <= p_ahora AND (m.hasta IS NULL OR m.hasta > p_ahora))
$$;
ALTER FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ) SECURITY INVOKER;
REVOKE ALL ON FUNCTION publicidad_habilitada(UUID, TIMESTAMPTZ) FROM dad_app;
DROP TABLE exclusiones_publicidad;

ALTER TABLE modulos_habilitados DROP CONSTRAINT modulos_habilitados_modulo_check;
ALTER TABLE modulos_habilitados ADD CONSTRAINT modulos_habilitados_modulo_check CHECK (modulo IN (
    'contabilidad', 'contabilidad_ph', 'facturacion_electronica', 'bienes_comunes',
    'control_acceso', 'asambleas', 'gestion_administrativa', 'publicidad'));
