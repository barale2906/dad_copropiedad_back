-- Búsqueda de proveedores desde el chat o la app, contacto autorizado y enlace público del
-- directorio para los grupos de WhatsApp (plan 08, fase 2; ADR 0014).
--
-- 1. Cada búsqueda queda en `solicitudes_proveedor` (por copropiedad): lo que se buscó (dato
--    personal, se borra a los 90 días), la categoría resuelta y las fichas sugeridas.
-- 2. Solo con un «Sí» explícito del residente (`autorizado_en`) se comparten su nombre y su
--    teléfono con esos proveedores y para esa solicitud; cada envío queda en
--    `contactos_proveedor` (append-only: es la prueba de la autorización y la base del cobro
--    `por_lead`).
-- 3. `enlaces_directorio`: token (solo su SHA-256) del enlace público del directorio de la PH.

-- ---------------------------------------------------------------------
-- Solicitudes de proveedor (por copropiedad)
-- ---------------------------------------------------------------------
CREATE TABLE solicitudes_proveedor (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    persona_id          UUID        NOT NULL,
    canal               TEXT        NOT NULL CHECK (canal IN ('whatsapp', 'app')),
    -- `wamid` en WhatsApp; en la app, una clave por petición. Reintentar no duplica.
    clave_idempotencia  TEXT        NOT NULL CHECK (char_length(clave_idempotencia) BETWEEN 1 AND 200),
    texto               TEXT        CHECK (char_length(texto) BETWEEN 1 AND 500),
    categoria           TEXT        REFERENCES categorias_publicidad (codigo),
    sugeridos           UUID[]      NOT NULL DEFAULT '{}' CHECK (cardinality(sugeridos) <= 3),
    autorizado_en       TIMESTAMPTZ,
    creada_en           TIMESTAMPTZ NOT NULL,
    texto_purgado_en    TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, clave_idempotencia),
    FOREIGN KEY (tenant_id, persona_id) REFERENCES personas (tenant_id, id),
    CHECK (texto IS NOT NULL OR texto_purgado_en IS NOT NULL),
    CHECK (autorizado_en IS NULL OR cardinality(sugeridos) > 0)
);
CREATE INDEX ix_solicitudes_proveedor_persona
    ON solicitudes_proveedor (tenant_id, persona_id, creada_en);
CREATE INDEX ix_solicitudes_proveedor_purga
    ON solicitudes_proveedor (creada_en) WHERE texto IS NOT NULL;
COMMENT ON COLUMN solicitudes_proveedor.texto IS
    'dato_personal: lo que escribió el residente al buscar; se borra a los 90 días';

-- Contactos compartidos con un proveedor (lead autorizado). Append-only.
CREATE TABLE contactos_proveedor (
    tenant_id      UUID        NOT NULL,
    solicitud_id   UUID        NOT NULL,
    anunciante_id  UUID        NOT NULL REFERENCES anunciantes (id),
    creado_en      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, solicitud_id, anunciante_id),
    FOREIGN KEY (tenant_id, solicitud_id) REFERENCES solicitudes_proveedor (tenant_id, id)
);
CREATE INDEX ix_contactos_proveedor_anunciante ON contactos_proveedor (anunciante_id, creado_en);

-- ---------------------------------------------------------------------
-- Enlace público del directorio (uno vigente por copropiedad)
-- ---------------------------------------------------------------------
CREATE TABLE enlaces_directorio (
    tenant_id   UUID        PRIMARY KEY REFERENCES copropiedades (id),
    -- SHA-256 del token: el token solo se entrega al generarlo.
    token_hash  BYTEA       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    creado_por  UUID        REFERENCES cuentas (id),
    creado_en   TIMESTAMPTZ NOT NULL
);

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER
-- ---------------------------------------------------------------------
-- Número visible de la línea fija de los enlaces `wa.me` (la primera activa): un enlace pegado
-- en un grupo no debe cambiar con la rotación de líneas. Cualquier línea atiende cualquier PH.
CREATE FUNCTION numero_whatsapp_publico()
RETURNS TEXT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT numero_visible FROM lineas_whatsapp WHERE estado = 'activa'
    ORDER BY creado_en, id LIMIT 1
$$;

-- Borra el texto de las búsquedas con más de `p_dias` días (nunca menos de 1) en todas las
-- copropiedades. La fila (categoría, sugeridos, autorización) se conserva como prueba.
CREATE FUNCTION purgar_solicitudes_proveedor(p_dias INTEGER)
RETURNS BIGINT
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH purgadas AS (
        UPDATE solicitudes_proveedor
        SET texto = NULL, texto_purgado_en = now()
        WHERE texto IS NOT NULL
          AND creada_en < now() - make_interval(days => greatest(p_dias, 1))
        RETURNING 1
    )
    SELECT count(*) FROM purgadas
$$;

REVOKE ALL ON FUNCTION numero_whatsapp_publico() FROM PUBLIC;
REVOKE ALL ON FUNCTION purgar_solicitudes_proveedor(INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION numero_whatsapp_publico() TO dad_app;
GRANT EXECUTE ON FUNCTION purgar_solicitudes_proveedor(INTEGER) TO dad_app;

-- ---------------------------------------------------------------------
-- Outbox: el contacto al proveedor y el «¿Le sirvió?» llevan su costo como `directorio`.
-- ---------------------------------------------------------------------
ALTER TABLE outbox DROP CONSTRAINT outbox_concepto_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_concepto_check
    CHECK (concepto IN ('control_acceso', 'comunicados', 'directorio'));

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['solicitudes_proveedor', 'contactos_proveedor', 'enlaces_directorio'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT ON solicitudes_proveedor, contactos_proveedor TO dad_app;
GRANT UPDATE (autorizado_en) ON solicitudes_proveedor TO dad_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON enlaces_directorio TO dad_app;

INSERT INTO permisos (codigo, ambito) VALUES ('directorio.compartir', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
SELECT uuid_rol_plantilla(r), 'copropiedad', 'directorio.compartir', NULL
FROM unnest(ARRAY['administrador', 'asistente_administrativo']) AS r;
