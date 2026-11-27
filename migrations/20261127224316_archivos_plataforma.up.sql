-- Archivos de la plataforma (no de una entidad): la imagen de cada creativo y el logo de cada
-- comercio. La clave va en la carpeta del UUID nulo (domain::ClaveArchivo::de_plataforma); solo
-- imágenes que muestra un navegador (domain::TipoArchivo::validar_imagen_publicidad).
CREATE TABLE archivos_plataforma (
    id               UUID        PRIMARY KEY,
    clave            TEXT        NOT NULL UNIQUE
                     CHECK (clave ~ '^00000000-0000-0000-0000-000000000000/[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f-]{36}$'),
    almacen          TEXT        NOT NULL CHECK (almacen IN ('local', 's3')),
    nombre_original  TEXT        NOT NULL CHECK (char_length(nombre_original) BETWEEN 1 AND 255),
    tipo_mime        TEXT        NOT NULL CHECK (tipo_mime IN ('image/jpeg', 'image/png', 'image/webp')),
    tamano_bytes     BIGINT      NOT NULL CHECK (tamano_bytes BETWEEN 1 AND 2097152),
    sha256           TEXT        NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    subido_por       UUID        REFERENCES cuentas (id),
    creado_en        TIMESTAMPTZ NOT NULL,
    CHECK (clave LIKE '%/' || id::text)
);
ALTER TABLE archivos_plataforma ENABLE ROW LEVEL SECURITY;
CREATE POLICY solo_plataforma ON archivos_plataforma
    USING (app_plataforma()) WITH CHECK (app_plataforma());
-- Los archivos no se editan: una imagen nueva es otro archivo.
GRANT SELECT, INSERT ON archivos_plataforma TO dad_app;

ALTER TABLE creativos ADD COLUMN imagen_archivo_id UUID REFERENCES archivos_plataforma (id);
ALTER TABLE anunciantes ADD COLUMN logo_archivo_id UUID REFERENCES archivos_plataforma (id);

-- Las piezas que ve una entidad llevan la imagen del creativo (id y tipo para firmar el enlace).
DROP FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ);
CREATE FUNCTION piezas_publicidad(p_formatos TEXT[], p_ahora TIMESTAMPTZ)
RETURNS TABLE (
    creativo_id UUID, campana_id UUID, anunciante_id UUID, nombre_comercial TEXT,
    anunciante_verificado BOOLEAN, formato TEXT, titulo TEXT, texto TEXT, boton TEXT,
    enlace TEXT, condiciones TEXT, es_promocion BOOLEAN, cupon_codigo TEXT,
    plantilla_whatsapp TEXT, estado_creativo TEXT, estado_campana TEXT, inicio TIMESTAMPTZ,
    fin TIMESTAMPTZ, modelo TEXT, tarifa NUMERIC, presupuesto_total NUMERIC,
    presupuesto_diario NUMERIC, consumido NUMERIC, consumido_hoy NUMERIC,
    impresiones_hoy BIGINT, imagen_archivo_id UUID, imagen_tipo TEXT)
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
              AND e.dia = dia_publicidad(p_ahora)),
           cr.imagen_archivo_id, ap.tipo_mime
    FROM creativos cr
    LEFT JOIN archivos_plataforma ap ON ap.id = cr.imagen_archivo_id
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
REVOKE ALL ON FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ) TO dad_app;
