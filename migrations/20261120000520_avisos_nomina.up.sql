-- Avisos automáticos de la nómina (plan 16): contratos a término fijo por vencer (preaviso de 30
-- días) y fechas legales (intereses de cesantías, consignación de cesantías, prima). Cada aviso se
-- envía una sola vez por referencia.

CREATE TABLE avisos_nomina (
    tenant_id   UUID        NOT NULL REFERENCES entidades (id),
    tipo        TEXT        NOT NULL CHECK (tipo IN (
                    'vencimiento_contrato', 'intereses_cesantias', 'consignacion_cesantias',
                    'prima')),
    -- Qué se avisó: el contrato y su fecha final, o el año (y el semestre) de la obligación.
    referencia  TEXT        NOT NULL CHECK (char_length(referencia) BETWEEN 1 AND 80),
    enviado_en  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, tipo, referencia)
);
ALTER TABLE avisos_nomina ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON avisos_nomina
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
CREATE TRIGGER avisos_nomina_solo_insercion BEFORE UPDATE OR DELETE ON avisos_nomina
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
GRANT SELECT, INSERT ON avisos_nomina TO dad_app;

-- El worker recorre las entidades con la nómina habilitada. Cruza tenants: solo los ids.
CREATE FUNCTION entidades_con_nomina(p_ahora TIMESTAMPTZ)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT m.tenant_id
    FROM modulos_habilitados m
    JOIN entidades e ON e.id = m.tenant_id AND e.estado = 'activa'
    WHERE m.modulo = 'nomina' AND m.desde <= p_ahora AND (m.hasta IS NULL OR m.hasta > p_ahora)
$$;
REVOKE ALL ON FUNCTION entidades_con_nomina(TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION entidades_con_nomina(TIMESTAMPTZ) TO dad_app;
