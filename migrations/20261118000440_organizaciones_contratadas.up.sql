-- Razón social y NIT de las organizaciones con contrato en la entidad de la transacción
-- (solicitud S12 del frontend). `organizaciones` es de plataforma: la entidad solo ve lo mínimo
-- de las que contrató. SECURITY DEFINER no aplica RLS, por eso se filtra por `app_tenant_id()`.
CREATE FUNCTION organizaciones_contratadas()
RETURNS TABLE (id UUID, razon_social TEXT, nit TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT o.id, o.razon_social, o.nit
    FROM organizaciones o
    WHERE app_tenant_id() IS NOT NULL
      AND EXISTS (SELECT 1 FROM contratos_servicio c
                  WHERE c.tenant_id = app_tenant_id() AND c.organizacion_id = o.id)
$$;
REVOKE ALL ON FUNCTION organizaciones_contratadas() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION organizaciones_contratadas() TO dad_app;
