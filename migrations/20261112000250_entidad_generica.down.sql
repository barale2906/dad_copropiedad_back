-- Revierte la entidad genérica: vuelve a `copropiedades` como tenant con todas sus columnas.

-- 3. Las tablas de la PH vuelven a apuntar al tenant
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT c.conrelid::regclass AS tabla, c.conname, pg_get_constraintdef(c.oid) AS def
        FROM pg_constraint c
        WHERE c.contype = 'f' AND c.confrelid = 'public.copropiedades'::regclass
          AND c.conparentid = 0
    LOOP
        EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I', r.tabla, r.conname);
        EXECUTE format('ALTER TABLE %s ADD CONSTRAINT %I %s', r.tabla, r.conname,
                       replace(r.def, 'REFERENCES copropiedades(id)', 'REFERENCES entidades(id)'));
    END LOOP;
END
$$;

-- 2. Las columnas de la PH vuelven al tenant
ALTER TABLE entidades
    ADD COLUMN tipo                   TEXT NOT NULL DEFAULT 'residencial'
                                      CHECK (tipo IN ('residencial', 'comercial', 'mixta')),
    ADD COLUMN forma                  TEXT NOT NULL DEFAULT 'conjunto'
                                      CHECK (forma IN ('edificio', 'conjunto', 'condominio',
                                                       'centro_comercial', 'parcelacion', 'otro')),
    ADD COLUMN estrato                SMALLINT CHECK (estrato BETWEEN 1 AND 6),
    ADD COLUMN escritura_numero       TEXT CHECK (char_length(escritura_numero) <= 40),
    ADD COLUMN notaria                TEXT CHECK (char_length(notaria) <= 120),
    ADD COLUMN escritura_fecha        DATE,
    ADD COLUMN matricula_matriz       TEXT CHECK (char_length(matricula_matriz) <= 40),
    ADD COLUMN reglamento_archivo_id  UUID,
    ADD COLUMN certificado_archivo_id UUID,
    ADD COLUMN plantilla_codigo       TEXT NOT NULL DEFAULT '{agrupacion}-{numero}'
                                      CHECK (char_length(plantilla_codigo) BETWEEN 8 AND 60
                                             AND plantilla_codigo LIKE '%{numero}%');

UPDATE entidades e SET
    tipo = c.tipo, forma = c.forma, estrato = c.estrato, escritura_numero = c.escritura_numero,
    notaria = c.notaria, escritura_fecha = c.escritura_fecha,
    matricula_matriz = c.matricula_matriz, reglamento_archivo_id = c.reglamento_archivo_id,
    certificado_archivo_id = c.certificado_archivo_id, plantilla_codigo = c.plantilla_codigo
FROM copropiedades c WHERE c.id = e.id;

-- Una entidad sin copropiedad no cabe en el esquema anterior.
DROP TABLE copropiedades;

-- 1. Nombre del tenant
ALTER TABLE entidades RENAME TO copropiedades;
ALTER INDEX entidades_pkey RENAME TO copropiedades_pkey;
ALTER INDEX entidades_nit_key RENAME TO copropiedades_nit_key;
ALTER TABLE copropiedades RENAME CONSTRAINT ck_entidades_nit TO ck_copropiedades_nit;
ALTER POLICY entidades_plataforma ON copropiedades RENAME TO copropiedades_plataforma;
COMMENT ON TABLE copropiedades IS NULL;

DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN SELECT conname FROM pg_constraint
             WHERE conrelid = 'public.copropiedades'::regclass AND conname LIKE 'entidades\_%'
    LOOP
        EXECUTE format('ALTER TABLE public.copropiedades RENAME CONSTRAINT %I TO %I',
                       r.conname, 'copropiedades' || substr(r.conname, length('entidades') + 1));
    END LOOP;
END
$$;

ALTER TABLE copropiedades
    ADD CONSTRAINT fk_copropiedades_reglamento
        FOREIGN KEY (id, reglamento_archivo_id) REFERENCES archivos (tenant_id, id),
    ADD CONSTRAINT fk_copropiedades_certificado
        FOREIGN KEY (id, certificado_archivo_id) REFERENCES archivos (tenant_id, id);

-- 4. Funciones como estaban
CREATE OR REPLACE FUNCTION public.alcance_campana(p_campana uuid)
 RETURNS bigint
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT count(*) FROM copropiedades p
    WHERE app_plataforma() AND p.estado = 'activa' AND publicidad_habilitada(p.id, now())
      AND campana_alcanza(p_campana, p.id)
$function$
;

CREATE OR REPLACE FUNCTION public.asistencias_por_telefono(p_telefono text)
 RETURNS TABLE(tenant_id uuid, copropiedad text, reunion_id uuid, titulo text, asistente_id uuid, linea text, confirmado boolean)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT a.tenant_id, c.nombre, r.id, r.titulo, a.id, a.linea_whatsapp,
           a.whatsapp_confirmado_en IS NOT NULL
    FROM asistentes a
    JOIN reuniones r     ON r.tenant_id = a.tenant_id AND r.id = a.reunion_id
    JOIN copropiedades c ON c.id = a.tenant_id
    WHERE a.telefono = p_telefono
      AND c.estado = 'activa'
      AND r.estado IN ('en_registro', 'instalada', 'suspendida')
      AND (r.plazo_respuesta IS NULL OR r.plazo_respuesta > now())
      AND EXISTS (SELECT 1 FROM modulos_habilitados m
                  WHERE m.tenant_id = c.id AND m.modulo = 'asambleas'
                    AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now()))
    ORDER BY r.fecha_programada, r.id
$function$
;

CREATE OR REPLACE FUNCTION public.campana_alcanza(p_campana uuid, p_tenant uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
 SET search_path TO 'pg_catalog', 'public'
AS $function$
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
$function$
;

CREATE OR REPLACE FUNCTION public.copropiedades_con_control_acceso()
 RETURNS TABLE(tenant_id uuid)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT DISTINCT m.tenant_id FROM modulos_habilitados m
    JOIN copropiedades c ON c.id = m.tenant_id AND c.estado = 'activa'
    WHERE m.modulo = 'control_acceso' AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now())
$function$
;

CREATE OR REPLACE FUNCTION public.copropiedades_con_publicidad(p_ahora timestamp with time zone)
 RETURNS TABLE(tenant_id uuid)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT c.id FROM copropiedades c
    WHERE c.estado = 'activa' AND publicidad_habilitada(c.id, p_ahora)
    ORDER BY c.id
$function$
;

CREATE OR REPLACE FUNCTION public.fichas_directorio_copropiedad(p_anunciante uuid, p_categoria text, p_texto text, p_solo_destacadas boolean, p_ahora timestamp with time zone, p_despues uuid, p_limite integer)
 RETURNS TABLE(anunciante_id uuid, nombre_comercial text, descripcion text, categorias text[], telefono text, whatsapp text, correo text, sitio_web text, horario text, direccion text, municipio_dane text, destacada boolean, distancia_km numeric, calificacion_promedio numeric, resenas bigint)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
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
$function$
;

CREATE OR REPLACE FUNCTION public.membresias_de_cuenta(p_cuenta uuid)
 RETURNS TABLE(tenant_id uuid, nombre text, roles text[])
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT c.id, c.nombre, array_agg(DISTINCT r.codigo ORDER BY r.codigo)
    FROM asignaciones_rol a
    JOIN copropiedades c ON c.id = a.tenant_id
    JOIN roles r         ON r.id = a.rol_id
    WHERE a.cuenta_id = p_cuenta
      AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
    GROUP BY c.id, c.nombre
    ORDER BY c.nombre
$function$
;

CREATE OR REPLACE FUNCTION public.puente_por_credencial(p_hash text)
 RETURNS TABLE(tenant_id uuid, puente_id uuid)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT p.tenant_id, p.id FROM puentes p
    JOIN copropiedades c ON c.id = p.tenant_id AND c.estado = 'activa'
    WHERE p.credencial_hash = p_hash AND p.estado = 'activo'
$function$
;

CREATE OR REPLACE FUNCTION public.resenas_reportadas(p_despues uuid, p_limite integer)
 RETURNS TABLE(resena_id uuid, tenant_id uuid, copropiedad text, anunciante_id uuid, nombre_comercial text, calificacion smallint, texto text, reportes bigint, motivos text[], creada_en timestamp with time zone)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
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
$function$
;

CREATE OR REPLACE FUNCTION public.tickets_de_cuenta(p_cuenta uuid, p_limite integer)
 RETURNS TABLE(tenant_id uuid, copropiedad text, ticket_id uuid, anio smallint, consecutivo integer, tipo text, estado text, asunto text, creado_en timestamp with time zone, actualizado_en timestamp with time zone)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    SELECT t.tenant_id, c.nombre, t.id, t.anio, t.consecutivo, t.tipo, t.estado, t.asunto,
           t.creado_en, t.actualizado_en
    FROM tickets t
    JOIN copropiedades c ON c.id = t.tenant_id AND c.estado = 'activa'
    WHERE t.solicitante_cuenta_id = p_cuenta
       OR t.solicitante_persona_id IN (SELECT p.id FROM personas p
                                       WHERE p.tenant_id = t.tenant_id AND p.cuenta_id = p_cuenta)
    ORDER BY t.actualizado_en DESC
    LIMIT least(greatest(p_limite, 1), 100)
$function$
;

CREATE OR REPLACE FUNCTION public.vinculos_por_telefono(p_telefono text)
 RETURNS TABLE(tenant_id uuid, copropiedad text, persona_id uuid, cuenta_id uuid, bien_privado_id uuid, codigo text, vinculo text)
 LANGUAGE sql
 STABLE SECURITY DEFINER
 SET search_path TO 'pg_catalog', 'public'
AS $function$
    WITH vinculos AS (
        SELECT t.tenant_id, t.persona_id, t.bien_privado_id, 'propietario' AS vinculo
        FROM titularidades t
        WHERE t.vigente_desde <= now() AND (t.vigente_hasta IS NULL OR t.vigente_hasta > now())
        UNION
        SELECT r.tenant_id, r.persona_id, r.bien_privado_id, 'residente'
        FROM residencias r
        WHERE r.vigente_desde <= now() AND (r.vigente_hasta IS NULL OR r.vigente_hasta > now())
    )
    SELECT DISTINCT ON (c.id, p.id, b.id)
           c.id, c.nombre, p.id, p.cuenta_id, b.id, b.codigo, v.vinculo
    FROM personas p
    JOIN vinculos v       ON v.tenant_id = p.tenant_id AND v.persona_id = p.id
    JOIN bienes_privados b ON b.tenant_id = v.tenant_id AND b.id = v.bien_privado_id
    JOIN copropiedades c  ON c.id = p.tenant_id
    WHERE p.telefono = p_telefono
      AND c.estado = 'activa'
      AND EXISTS (SELECT 1 FROM modulos_habilitados m
                  WHERE m.tenant_id = c.id AND m.modulo = 'gestion_administrativa'
                    AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now()))
    ORDER BY c.id, p.id, b.id, v.vinculo
$function$
;
