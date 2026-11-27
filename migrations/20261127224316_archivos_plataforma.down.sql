DROP FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ);
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
REVOKE ALL ON FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION piezas_publicidad(TEXT[], TIMESTAMPTZ) TO dad_app;

ALTER TABLE anunciantes DROP COLUMN logo_archivo_id;
ALTER TABLE creativos DROP COLUMN imagen_archivo_id;
DROP TABLE archivos_plataforma;
