-- Entidades de la cuenta al iniciar sesión (ADR 0016, adenda del 2026-11-14).
--
-- Una persona entra a gestionar una propiedad horizontal, una empresa cliente o la plataforma
-- misma. La plataforma también es una empresa (GISLA SAS: factura, lleva su contabilidad), pero
-- tiene esa función: se marca con `es_plataforma` (una sola entidad, que no puede ser una PH). La
-- lista que recibe el cliente al entrar dice el tipo de cada entidad para que sepa qué mostrar, e
-- incluye la plataforma para quien tiene roles de plataforma. Las retiradas no aparecen.

ALTER TABLE entidades ADD COLUMN es_plataforma BOOLEAN NOT NULL DEFAULT FALSE;
CREATE UNIQUE INDEX ux_entidad_plataforma ON entidades ((TRUE)) WHERE es_plataforma;
COMMENT ON COLUMN entidades.es_plataforma IS
    'la entidad que opera la plataforma (una sola; nunca una copropiedad)';

-- La plataforma no es una propiedad horizontal.
CREATE FUNCTION exigir_plataforma_no_copropiedad() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    choca BOOLEAN;
BEGIN
    -- Cada tabla por separado: PL/pgSQL no corta la evaluación y `copropiedades` no tiene
    -- `es_plataforma`.
    IF TG_TABLE_NAME = 'copropiedades' THEN
        choca := EXISTS (SELECT 1 FROM entidades WHERE id = NEW.id AND es_plataforma);
    ELSE
        choca := NEW.es_plataforma AND EXISTS (SELECT 1 FROM copropiedades WHERE id = NEW.id);
    END IF;
    IF choca THEN
        RAISE EXCEPTION 'la entidad de la plataforma no puede ser una copropiedad'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_copropiedad_no_plataforma BEFORE INSERT ON copropiedades
    FOR EACH ROW EXECUTE FUNCTION exigir_plataforma_no_copropiedad();
CREATE TRIGGER trg_plataforma_no_copropiedad BEFORE INSERT OR UPDATE OF es_plataforma ON entidades
    FOR EACH ROW EXECUTE FUNCTION exigir_plataforma_no_copropiedad();

-- Designar la entidad de la plataforma cruza tenants: solo por esta función, que llama el caso
-- de uso después de exigir el permiso. Devuelve FALSE si la entidad no existe o está retirada.
CREATE FUNCTION designar_entidad_plataforma(p_entidad UUID) RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM entidades WHERE id = p_entidad AND estado <> 'retirada') THEN
        RETURN FALSE;
    END IF;
    UPDATE entidades SET es_plataforma = FALSE WHERE es_plataforma AND id <> p_entidad;
    UPDATE entidades SET es_plataforma = TRUE WHERE id = p_entidad AND NOT es_plataforma;
    RETURN TRUE;
END
$$;

-- Tipo de la entidad de la transacción (capacidades).
CREATE FUNCTION tipo_entidad(p_entidad UUID) RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT CASE
        WHEN e.es_plataforma THEN 'plataforma'
        WHEN EXISTS (SELECT 1 FROM copropiedades c WHERE c.id = e.id) THEN 'propiedad_horizontal'
        ELSE 'empresa' END
    FROM entidades e WHERE e.id = p_entidad
$$;

DROP FUNCTION membresias_de_cuenta(uuid);

-- Roles de la cuenta en cada entidad; en la de la plataforma se suman sus roles de plataforma.
CREATE FUNCTION membresias_de_cuenta(p_cuenta UUID)
RETURNS TABLE (tenant_id UUID, nombre TEXT, nit TEXT, tipo TEXT, estado TEXT, roles TEXT[])
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT e.id, e.nombre, e.nit, tipo_entidad(e.id), e.estado,
           array_agg(DISTINCT x.rol ORDER BY x.rol)
    FROM (
        SELECT a.tenant_id AS entidad, r.codigo AS rol
        FROM asignaciones_rol a JOIN roles r ON r.id = a.rol_id
        WHERE a.cuenta_id = p_cuenta
          AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
        UNION ALL
        SELECT p.id, r.codigo
        FROM asignaciones_plataforma ap
        JOIN roles r     ON r.id = ap.rol_id
        JOIN entidades p ON p.es_plataforma
        WHERE ap.cuenta_id = p_cuenta
          AND ap.vigente_desde <= now() AND (ap.vigente_hasta IS NULL OR ap.vigente_hasta > now())
    ) x
    JOIN entidades e ON e.id = x.entidad
    WHERE e.estado <> 'retirada'
    GROUP BY e.id, e.nombre, e.nit, e.estado, e.es_plataforma
    ORDER BY e.es_plataforma DESC, normalizar(e.nombre), e.id
$$;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY['membresias_de_cuenta(uuid)', 'designar_entidad_plataforma(uuid)',
                             'tipo_entidad(uuid)', 'exigir_plataforma_no_copropiedad()'] LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f);
    END LOOP;
END $$;
GRANT EXECUTE ON FUNCTION membresias_de_cuenta(uuid), designar_entidad_plataforma(uuid),
    tipo_entidad(uuid) TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES ('plataforma.entidad_propia.designar', 'plataforma');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
VALUES (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.entidad_propia.designar', NULL);
