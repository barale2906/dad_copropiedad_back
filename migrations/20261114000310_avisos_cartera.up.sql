-- Avisos de la cartera de la PH (plan 14 fase 6): recordatorio de pago antes del vencimiento y
-- aviso de mora después, una sola vez por cuenta de cobro y clase (frecuencia limitada).

CREATE TABLE avisos_cartera (
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    cuenta_cobro_id  UUID        NOT NULL,
    tipo             TEXT        NOT NULL CHECK (tipo IN ('recordatorio', 'mora')),
    enviado_en       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (cuenta_cobro_id, tipo),
    FOREIGN KEY (tenant_id, cuenta_cobro_id) REFERENCES cuentas_cobro (tenant_id, id)
);
ALTER TABLE avisos_cartera ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON avisos_cartera
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
GRANT SELECT, INSERT ON avisos_cartera TO dad_app;
CREATE TRIGGER avisos_cartera_solo_insercion BEFORE UPDATE OR DELETE ON avisos_cartera
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();

-- Copropiedades con la cartera habilitada que tienen cuentas de cobro por recordar (vencen en los
-- próximos `p_dias_antes` días) o por avisar en mora (vencieron hace `p_dias_despues` días o más,
-- dentro de los últimos 60), sin aviso de esa clase. Solo devuelve ids de tenant: el worker
-- revisa cada una en su propia transacción.
CREATE FUNCTION copropiedades_con_avisos_cartera(p_hoy DATE, p_dias_antes INTEGER,
                                                 p_dias_despues INTEGER)
RETURNS TABLE (tenant_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT DISTINCT c.tenant_id
    FROM cuentas_cobro c
    JOIN liquidaciones_expensas l ON l.id = c.liquidacion_id AND l.estado = 'contabilizada'
    JOIN modulos_habilitados m ON m.tenant_id = c.tenant_id AND m.modulo = 'contabilidad_ph'
        AND m.desde <= now() AND (m.hasta IS NULL OR m.hasta > now())
    WHERE (c.vencimiento > p_hoy AND c.vencimiento <= p_hoy + p_dias_antes
           AND NOT EXISTS (SELECT 1 FROM avisos_cartera a
                           WHERE a.cuenta_cobro_id = c.id AND a.tipo = 'recordatorio'))
       OR (c.vencimiento <= p_hoy - p_dias_despues AND c.vencimiento > p_hoy - 60
           AND NOT EXISTS (SELECT 1 FROM avisos_cartera a
                           WHERE a.cuenta_cobro_id = c.id AND a.tipo = 'mora'))
$$;
REVOKE ALL ON FUNCTION copropiedades_con_avisos_cartera(DATE, INTEGER, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION copropiedades_con_avisos_cartera(DATE, INTEGER, INTEGER) TO dad_app;
