-- Módulos contratados por copropiedad (plan 01 §9): permite vender por suscripción y por
-- módulo. Los módulos base (identidad, copropiedad y bienes privados) siempre están activos y
-- no se registran aquí.
CREATE TABLE modulos_habilitados (
    id         UUID        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES copropiedades (id),
    modulo     TEXT        NOT NULL CHECK (modulo IN (
                   'bienes_comunes', 'control_acceso', 'asambleas', 'gestion_administrativa',
                   'publicidad')),
    desde      TIMESTAMPTZ NOT NULL,
    hasta      TIMESTAMPTZ CHECK (hasta > desde),   -- NULL = sin fecha de fin
    creado_en  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_modulos_habilitados ON modulos_habilitados (tenant_id, modulo, desde);

ALTER TABLE modulos_habilitados ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON modulos_habilitados
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());

-- Habilitar = nuevo periodo; terminar = fijar `hasta`. Nunca se borra el historial (facturación).
GRANT SELECT, INSERT ON modulos_habilitados TO dad_app;
GRANT UPDATE (hasta) ON modulos_habilitados TO dad_app;
