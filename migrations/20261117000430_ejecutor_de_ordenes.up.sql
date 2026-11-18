-- Cuatro ojos en las órdenes de trabajo (invariante del ADR 0009): quien ejecuta una orden no
-- la verifica. Se guarda quién la ejecutó y la base de datos lo vuelve a exigir.
ALTER TABLE ordenes_trabajo ADD COLUMN ejecutada_por UUID;
ALTER TABLE ordenes_trabajo ADD CONSTRAINT ck_orden_verifica_otra_cuenta
    CHECK (verificada_por IS NULL OR ejecutada_por IS NULL OR verificada_por <> ejecutada_por);
COMMENT ON COLUMN ordenes_trabajo.ejecutada_por IS 'Cuenta que registró la ejecución; no puede verificarla';
