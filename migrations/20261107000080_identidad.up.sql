-- Identidad, roles y permisos (plan 02).
--
-- Ámbitos de las transacciones de la aplicación (`dad_app`):
--   * de copropiedad: `app.tenant_id` fijado → RLS por tenant (como hasta ahora);
--   * de plataforma: `app.plataforma = 'on'` → cuentas, sesiones, roles de plataforma,
--     organizaciones. Una transacción de copropiedad NO ve las cuentas ni las sesiones.
-- Lo que cruza ámbitos va por funciones SECURITY DEFINER que devuelven lo mínimo.

CREATE FUNCTION app_plataforma() RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT coalesce(current_setting('app.plataforma', true), '') = 'on'
$$;

-- ---------------------------------------------------------------------
-- Cuentas y sesiones (plataforma)
-- ---------------------------------------------------------------------
CREATE TABLE cuentas (
    id                         UUID        PRIMARY KEY,
    nombre_mostrar             TEXT        NOT NULL CHECK (char_length(nombre_mostrar) BETWEEN 1 AND 120),
    usuario                    TEXT        UNIQUE CHECK (usuario ~ '^[a-z][a-z0-9._-]{2,29}$'),
    correo                     TEXT        UNIQUE CHECK (correo = lower(correo) AND correo LIKE '%_@_%'),
    correo_verificado_en       TIMESTAMPTZ,
    estado                     TEXT        NOT NULL DEFAULT 'activa'
                               CHECK (estado IN ('activa', 'bloqueada', 'eliminada')),
    hash_contrasena            TEXT        NOT NULL,
    contrasena_actualizada_en  TIMESTAMPTZ NOT NULL,
    intentos_fallidos          INTEGER     NOT NULL DEFAULT 0 CHECK (intentos_fallidos >= 0),
    bloqueo_hasta              TIMESTAMPTZ,
    totp_secreto               TEXT,
    totp_activado_en           TIMESTAMPTZ,
    creada_en                  TIMESTAMPTZ NOT NULL,
    ultimo_acceso_en           TIMESTAMPTZ,
    -- Se inicia sesión con usuario o correo: al menos uno.
    CHECK (usuario IS NOT NULL OR correo IS NOT NULL),
    CHECK (totp_activado_en IS NULL OR totp_secreto IS NOT NULL)
);
COMMENT ON COLUMN cuentas.nombre_mostrar IS 'dato_personal: nombre de la persona';
COMMENT ON COLUMN cuentas.usuario IS 'dato_personal: nombre de usuario';
COMMENT ON COLUMN cuentas.correo IS 'dato_personal: correo electrónico';
COMMENT ON COLUMN cuentas.hash_contrasena IS 'dato_sensible: hash Argon2id de la contraseña';
COMMENT ON COLUMN cuentas.totp_secreto IS 'dato_sensible: secreto del segundo factor';

-- Códigos de un solo uso (verificar correo, recuperar contraseña). Solo el hash.
CREATE TABLE codigos_verificacion (
    id           UUID        PRIMARY KEY,
    cuenta_id    UUID        NOT NULL REFERENCES cuentas (id),
    proposito    TEXT        NOT NULL CHECK (proposito IN ('verificar_correo', 'recuperar_contrasena')),
    hash_codigo  TEXT        NOT NULL CHECK (hash_codigo ~ '^[0-9a-f]{64}$'),
    expira_en    TIMESTAMPTZ NOT NULL,
    intentos     INTEGER     NOT NULL DEFAULT 0 CHECK (intentos >= 0),
    usado_en     TIMESTAMPTZ,
    creado_en    TIMESTAMPTZ NOT NULL
);
CREATE INDEX ix_codigos_cuenta ON codigos_verificacion (cuenta_id, proposito, creado_en DESC);

-- Sesión por dispositivo. Tokens opacos: solo se guarda su SHA-256.
CREATE TABLE sesiones (
    id                         UUID        PRIMARY KEY,
    cuenta_id                  UUID        NOT NULL REFERENCES cuentas (id),
    hash_acceso                TEXT        NOT NULL UNIQUE,
    acceso_expira_en           TIMESTAMPTZ NOT NULL,
    hash_refresh               TEXT        UNIQUE,
    -- Refresh ya usado: si vuelve a llegar, alguien lo copió → se revoca la sesión.
    hash_refresh_anterior      TEXT,
    expira_maximo_en           TIMESTAMPTZ NOT NULL,
    segundo_factor_pendiente   BOOLEAN     NOT NULL DEFAULT FALSE,
    segundo_factor_verificado  BOOLEAN     NOT NULL DEFAULT FALSE,
    equipo_compartido          BOOLEAN     NOT NULL DEFAULT FALSE,
    agente                     TEXT        CHECK (char_length(agente) <= 200),
    creada_en                  TIMESTAMPTZ NOT NULL,
    renovada_en                TIMESTAMPTZ,
    revocada_en                TIMESTAMPTZ,
    motivo_revocacion          TEXT
);
CREATE INDEX ix_sesiones_cuenta ON sesiones (cuenta_id) WHERE revocada_en IS NULL;
CREATE INDEX ix_sesiones_refresh_anterior ON sesiones (hash_refresh_anterior)
    WHERE hash_refresh_anterior IS NOT NULL;

-- Auditoría de lo que no pertenece a una copropiedad (cuentas, roles de plataforma, altas).
CREATE TABLE auditoria_plataforma (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ocurrido_en  TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_id     UUID,
    accion       TEXT        NOT NULL,
    entidad      TEXT        NOT NULL,
    entidad_id   UUID,
    resultado    TEXT        NOT NULL CHECK (resultado IN ('ok', 'rechazado', 'error')),
    detalle      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    request_id   TEXT
);
CREATE INDEX ix_auditoria_plataforma_entidad ON auditoria_plataforma (entidad, entidad_id, id);
CREATE TRIGGER trg_auditoria_plataforma_inmutable BEFORE UPDATE OR DELETE ON auditoria_plataforma
    FOR EACH ROW EXECUTE FUNCTION rechazar_modificacion();
CREATE TRIGGER trg_auditoria_plataforma_sin_truncate BEFORE TRUNCATE ON auditoria_plataforma
    FOR EACH STATEMENT EXECUTE FUNCTION rechazar_modificacion();

-- Los mensajes de plataforma (recuperar contraseña…) no tienen copropiedad.
ALTER TABLE outbox ALTER COLUMN tenant_id DROP NOT NULL;
CREATE POLICY outbox_plataforma ON outbox
    USING (tenant_id IS NULL AND app_plataforma())
    WITH CHECK (tenant_id IS NULL AND app_plataforma());

-- ---------------------------------------------------------------------
-- Permisos y roles
-- ---------------------------------------------------------------------
CREATE TABLE permisos (
    codigo  TEXT PRIMARY KEY,
    ambito  TEXT NOT NULL CHECK (ambito IN ('plataforma', 'copropiedad')),
    UNIQUE (codigo, ambito)
);

-- tenant_id NULL = plantilla del sistema o rol de plataforma; con tenant = rol propio de la PH.
CREATE TABLE roles (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        REFERENCES copropiedades (id),
    codigo        TEXT        NOT NULL CHECK (codigo ~ '^[a-z][a-z0-9_]{1,59}$'),
    nombre        TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 80),
    descripcion   TEXT,
    ambito        TEXT        NOT NULL CHECK (ambito IN ('plataforma', 'copropiedad')),
    es_plantilla  BOOLEAN     NOT NULL DEFAULT FALSE,
    creado_en     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (tenant_id, codigo),
    UNIQUE (id, ambito),
    -- Los roles de plataforma no pertenecen a una copropiedad.
    CHECK (ambito = 'copropiedad' OR tenant_id IS NULL),
    CHECK (NOT es_plantilla OR tenant_id IS NULL)
);

-- Las FK compuestas con `ambito` impiden dar a un rol un permiso de otro ámbito.
CREATE TABLE roles_permisos (
    rol_id     UUID NOT NULL,
    ambito     TEXT NOT NULL,
    permiso    TEXT NOT NULL,
    tenant_id  UUID,      -- copia del tenant del rol (para RLS)
    PRIMARY KEY (rol_id, permiso),
    FOREIGN KEY (rol_id, ambito)  REFERENCES roles (id, ambito) ON DELETE CASCADE,
    FOREIGN KEY (permiso, ambito) REFERENCES permisos (codigo, ambito)
);

-- Determinista para poder referirse a una plantilla en migraciones futuras.
CREATE FUNCTION uuid_rol_plantilla(p_codigo TEXT) RETURNS uuid
LANGUAGE sql IMMUTABLE AS $$ SELECT md5('rol_plantilla:' || p_codigo)::uuid $$;

-- Asignaciones de roles de copropiedad (por tenant).
CREATE TABLE asignaciones_rol (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES copropiedades (id),
    cuenta_id      UUID        NOT NULL REFERENCES cuentas (id),
    rol_id         UUID        NOT NULL,
    ambito         TEXT        NOT NULL DEFAULT 'copropiedad' CHECK (ambito = 'copropiedad'),
    vigente_desde  TIMESTAMPTZ NOT NULL,
    vigente_hasta  TIMESTAMPTZ CHECK (vigente_hasta >= vigente_desde),
    origen         TEXT        NOT NULL CHECK (origen IN (
                       'manual', 'contrato_servicio', 'propiedad', 'residencia', 'asamblea')),
    origen_id      UUID,
    asignado_por   UUID,
    creada_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (rol_id, ambito) REFERENCES roles (id, ambito),
    CHECK ((origen = 'manual') = (origen_id IS NULL))
);
CREATE INDEX ix_asignaciones_cuenta ON asignaciones_rol (tenant_id, cuenta_id, vigente_desde);
CREATE INDEX ix_asignaciones_origen ON asignaciones_rol (tenant_id, origen, origen_id)
    WHERE origen_id IS NOT NULL;
CREATE INDEX ix_asignaciones_cuenta_global ON asignaciones_rol (cuenta_id);

-- El rol debe ser plantilla o de la misma copropiedad (RLS oculta los de otras PH).
CREATE FUNCTION validar_asignacion_rol() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM roles r
        WHERE r.id = NEW.rol_id AND (r.tenant_id IS NULL OR r.tenant_id = NEW.tenant_id)
    ) THEN
        RAISE EXCEPTION 'el rol % no pertenece a la copropiedad', NEW.rol_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_asignaciones_rol_validar BEFORE INSERT ON asignaciones_rol
    FOR EACH ROW EXECUTE FUNCTION validar_asignacion_rol();

-- Roles de plataforma (superusuario, soporte, comercial…).
CREATE TABLE asignaciones_plataforma (
    id             UUID        PRIMARY KEY,
    cuenta_id      UUID        NOT NULL REFERENCES cuentas (id),
    rol_id         UUID        NOT NULL,
    ambito         TEXT        NOT NULL DEFAULT 'plataforma' CHECK (ambito = 'plataforma'),
    vigente_desde  TIMESTAMPTZ NOT NULL,
    vigente_hasta  TIMESTAMPTZ CHECK (vigente_hasta >= vigente_desde),
    asignado_por   UUID,
    creada_en      TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (rol_id, ambito) REFERENCES roles (id, ambito)
);
CREATE INDEX ix_asignaciones_plataforma_cuenta ON asignaciones_plataforma (cuenta_id);

-- ---------------------------------------------------------------------
-- Organizaciones (plataforma) y contratos de servicio (copropiedad)
-- ---------------------------------------------------------------------
CREATE TABLE organizaciones (
    id                  UUID        PRIMARY KEY,
    nit                 TEXT        NOT NULL UNIQUE CHECK (nit ~ '^[1-9][0-9]{0,14}-[0-9]$'),
    razon_social        TEXT        NOT NULL CHECK (char_length(razon_social) BETWEEN 1 AND 200),
    naturaleza          TEXT        NOT NULL CHECK (naturaleza IN ('juridica', 'natural_con_rut')),
    tipos               TEXT[]      NOT NULL CHECK (
                            cardinality(tipos) > 0 AND tipos <@ ARRAY[
                                'administradora', 'contable', 'revisoria_fiscal', 'seguridad',
                                'aseo', 'mantenimiento', 'anunciante', 'otro']),
    correo_contacto     TEXT,
    telefono_contacto   TEXT,
    estado              TEXT        NOT NULL DEFAULT 'sin_reclamar'
                        CHECK (estado IN ('sin_reclamar', 'reclamada', 'suspendida')),
    creada_por_cuenta   UUID        REFERENCES cuentas (id),
    creada_en_tenant    UUID        REFERENCES copropiedades (id),
    creada_en           TIMESTAMPTZ NOT NULL DEFAULT now(),
    actualizada_en      TIMESTAMPTZ NOT NULL DEFAULT now()
);
COMMENT ON COLUMN organizaciones.nit IS 'dato_personal: en personas naturales con RUT es la cédula';

CREATE TABLE organizaciones_miembros (
    organizacion_id  UUID        NOT NULL REFERENCES organizaciones (id),
    cuenta_id        UUID        NOT NULL REFERENCES cuentas (id),
    rol              TEXT        NOT NULL CHECK (rol IN ('dueno', 'gestor', 'operario')),
    creado_en        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organizacion_id, cuenta_id)
);
CREATE INDEX ix_miembros_cuenta ON organizaciones_miembros (cuenta_id);

CREATE TABLE reclamaciones_organizacion (
    id               UUID        PRIMARY KEY,
    organizacion_id  UUID        NOT NULL REFERENCES organizaciones (id),
    cuenta_id        UUID        NOT NULL REFERENCES cuentas (id),
    mensaje          TEXT        NOT NULL CHECK (char_length(mensaje) BETWEEN 1 AND 2000),
    estado           TEXT        NOT NULL DEFAULT 'pendiente'
                     CHECK (estado IN ('pendiente', 'aprobada', 'rechazada')),
    creada_en        TIMESTAMPTZ NOT NULL,
    revisada_por     UUID        REFERENCES cuentas (id),
    revisada_en      TIMESTAMPTZ,
    motivo           TEXT
);
CREATE UNIQUE INDEX ux_reclamacion_pendiente ON reclamaciones_organizacion (organizacion_id, cuenta_id)
    WHERE estado = 'pendiente';

CREATE TABLE contratos_servicio (
    id                  UUID          PRIMARY KEY,
    tenant_id           UUID          NOT NULL REFERENCES copropiedades (id),
    organizacion_id     UUID          NOT NULL REFERENCES organizaciones (id),
    tipo_servicio       TEXT          NOT NULL CHECK (tipo_servicio IN (
                            'administradora', 'contable', 'revisoria_fiscal', 'seguridad',
                            'aseo', 'mantenimiento', 'anunciante', 'otro')),
    vigente_desde       TIMESTAMPTZ   NOT NULL,
    vigente_hasta       TIMESTAMPTZ   CHECK (vigente_hasta >= vigente_desde),
    estado              TEXT          NOT NULL DEFAULT 'vigente' CHECK (estado IN ('vigente', 'terminado')),
    valor               NUMERIC(16,2) CHECK (valor >= 0),
    soporte_archivo_id  UUID,
    creado_en           TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, soporte_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_contratos_organizacion ON contratos_servicio (tenant_id, organizacion_id);

-- ---------------------------------------------------------------------
-- RLS
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'cuentas', 'codigos_verificacion', 'sesiones', 'auditoria_plataforma',
        'asignaciones_plataforma', 'organizaciones', 'organizaciones_miembros',
        'reclamaciones_organizacion'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY solo_plataforma ON %I USING (app_plataforma()) WITH CHECK (app_plataforma())', t);
    END LOOP;
    FOREACH t IN ARRAY ARRAY['asignaciones_rol', 'contratos_servicio'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

-- Roles: las plantillas y los de plataforma se ven desde cualquier lado; los propios de una PH,
-- solo en ella. Las plantillas no se modifican desde la aplicación.
ALTER TABLE roles ENABLE ROW LEVEL SECURITY;
CREATE POLICY roles_ver ON roles FOR SELECT
    USING (tenant_id IS NULL OR tenant_id = app_tenant_id());
CREATE POLICY roles_crear ON roles FOR INSERT
    WITH CHECK (NOT es_plantilla AND (tenant_id = app_tenant_id()
                OR (tenant_id IS NULL AND ambito = 'plataforma' AND app_plataforma())));
CREATE POLICY roles_editar ON roles FOR UPDATE
    USING (NOT es_plantilla AND (tenant_id = app_tenant_id() OR (tenant_id IS NULL AND app_plataforma())))
    WITH CHECK (NOT es_plantilla AND (tenant_id = app_tenant_id() OR (tenant_id IS NULL AND app_plataforma())));

ALTER TABLE roles_permisos ENABLE ROW LEVEL SECURITY;
CREATE POLICY roles_permisos_ver ON roles_permisos FOR SELECT
    USING (tenant_id IS NULL OR tenant_id = app_tenant_id());
CREATE POLICY roles_permisos_escribir ON roles_permisos FOR ALL
    USING (EXISTS (SELECT 1 FROM roles r WHERE r.id = rol_id AND NOT r.es_plantilla
                   AND (r.tenant_id = app_tenant_id() OR (r.tenant_id IS NULL AND app_plataforma()))))
    WITH CHECK (EXISTS (SELECT 1 FROM roles r WHERE r.id = rol_id AND NOT r.es_plantilla
                        AND r.tenant_id IS NOT DISTINCT FROM roles_permisos.tenant_id
                        AND (r.tenant_id = app_tenant_id() OR (r.tenant_id IS NULL AND app_plataforma()))));

-- La plataforma lista y da de alta copropiedades.
CREATE POLICY copropiedades_plataforma ON copropiedades FOR SELECT USING (app_plataforma());

-- ---------------------------------------------------------------------
-- Funciones SECURITY DEFINER (cruzan ámbitos; devuelven lo mínimo)
-- ---------------------------------------------------------------------
-- Copropiedades donde la cuenta tiene roles vigentes, con los códigos de esos roles.
CREATE FUNCTION membresias_de_cuenta(p_cuenta UUID)
RETURNS TABLE (tenant_id UUID, nombre TEXT, roles TEXT[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT c.id, c.nombre, array_agg(DISTINCT r.codigo ORDER BY r.codigo)
    FROM asignaciones_rol a
    JOIN copropiedades c ON c.id = a.tenant_id
    JOIN roles r         ON r.id = a.rol_id
    WHERE a.cuenta_id = p_cuenta
      AND a.vigente_desde <= now() AND (a.vigente_hasta IS NULL OR a.vigente_hasta > now())
    GROUP BY c.id, c.nombre
    ORDER BY c.nombre
$$;

-- Busca una cuenta por usuario o correo exactos (para asignarle un rol). Solo id y nombre.
CREATE FUNCTION buscar_cuenta(p_identificador TEXT)
RETURNS TABLE (id UUID, nombre_mostrar TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT id, nombre_mostrar FROM cuentas
    WHERE estado = 'activa' AND (usuario = lower(p_identificador) OR correo = lower(p_identificador))
$$;

-- Ficha pública de una organización por NIT exacto (nunca listados: puede ser una cédula).
CREATE FUNCTION buscar_organizacion_por_nit(p_nit TEXT)
RETURNS TABLE (id UUID, razon_social TEXT, naturaleza TEXT, tipos TEXT[], estado TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT id, razon_social, naturaleza, tipos, estado FROM organizaciones WHERE nit = p_nit
$$;

-- Crea la organización desde la copropiedad de la transacción, o devuelve la existente con
-- ese NIT (dos PH creándola a la vez → una sola fila).
CREATE FUNCTION crear_organizacion(
    p_id UUID, p_nit TEXT, p_razon_social TEXT, p_naturaleza TEXT, p_tipos TEXT[],
    p_correo TEXT, p_telefono TEXT, p_cuenta UUID)
RETURNS TABLE (id UUID, creada BOOLEAN)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_tenant UUID := app_tenant_id();
BEGIN
    IF v_tenant IS NULL THEN
        RAISE EXCEPTION 'crear_organizacion exige una copropiedad en la transacción';
    END IF;
    INSERT INTO organizaciones AS o (id, nit, razon_social, naturaleza, tipos, correo_contacto,
                                     telefono_contacto, creada_por_cuenta, creada_en_tenant)
    VALUES (p_id, p_nit, p_razon_social, p_naturaleza, p_tipos, p_correo, p_telefono, p_cuenta, v_tenant)
    ON CONFLICT (nit) DO NOTHING;
    RETURN QUERY SELECT o.id, o.id = p_id FROM organizaciones o WHERE o.nit = p_nit;
END
$$;

-- Organizaciones de las que la cuenta es miembro (para el personal de una empresa).
CREATE FUNCTION organizaciones_de_cuenta(p_cuenta UUID)
RETURNS TABLE (organizacion_id UUID, rol TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT organizacion_id, rol FROM organizaciones_miembros WHERE cuenta_id = p_cuenta
$$;

-- ---------------------------------------------------------------------
-- Permisos de la aplicación
-- ---------------------------------------------------------------------
GRANT SELECT, INSERT ON cuentas TO dad_app;
GRANT UPDATE (nombre_mostrar, correo, correo_verificado_en, estado, hash_contrasena,
              contrasena_actualizada_en, intentos_fallidos, bloqueo_hasta, totp_secreto,
              totp_activado_en, ultimo_acceso_en) ON cuentas TO dad_app;
GRANT SELECT, INSERT ON codigos_verificacion TO dad_app;
GRANT UPDATE (intentos, usado_en) ON codigos_verificacion TO dad_app;
GRANT SELECT, INSERT ON sesiones TO dad_app;
GRANT UPDATE (hash_acceso, acceso_expira_en, hash_refresh, hash_refresh_anterior,
              segundo_factor_pendiente, segundo_factor_verificado, renovada_en, revocada_en,
              motivo_revocacion) ON sesiones TO dad_app;
GRANT SELECT, INSERT ON auditoria_plataforma TO dad_app;
GRANT USAGE ON SEQUENCE auditoria_plataforma_id_seq TO dad_app;
GRANT SELECT ON permisos TO dad_app;
GRANT SELECT, INSERT ON roles TO dad_app;
GRANT UPDATE (nombre, descripcion) ON roles TO dad_app;
GRANT SELECT, INSERT, DELETE ON roles_permisos TO dad_app;
GRANT SELECT, INSERT ON asignaciones_rol, asignaciones_plataforma TO dad_app;
GRANT UPDATE (vigente_hasta) ON asignaciones_rol, asignaciones_plataforma TO dad_app;
GRANT SELECT, INSERT ON organizaciones, organizaciones_miembros, reclamaciones_organizacion TO dad_app;
GRANT UPDATE (razon_social, tipos, correo_contacto, telefono_contacto, estado, actualizada_en)
    ON organizaciones TO dad_app;
GRANT UPDATE (rol) ON organizaciones_miembros TO dad_app;
GRANT UPDATE (estado, revisada_por, revisada_en, motivo) ON reclamaciones_organizacion TO dad_app;
GRANT SELECT, INSERT ON contratos_servicio TO dad_app;
GRANT UPDATE (estado, vigente_hasta) ON contratos_servicio TO dad_app;
GRANT INSERT ON copropiedades TO dad_app;

DO $$
DECLARE
    f TEXT;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'membresias_de_cuenta(uuid)', 'buscar_cuenta(text)', 'buscar_organizacion_por_nit(text)',
        'crear_organizacion(uuid, text, text, text, text[], text, text, uuid)',
        'organizaciones_de_cuenta(uuid)'
    ] LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f);
        EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO dad_app', f);
    END LOOP;
END
$$;

-- ---------------------------------------------------------------------
-- Catálogo inicial
-- ---------------------------------------------------------------------
-- Catálogo de permisos (= domain::Permiso). Generado del código; la prueba
-- `catalogo_de_permisos_coincide_con_el_codigo` compara ambos.
INSERT INTO permisos (codigo, ambito) VALUES
    ('plataforma.copropiedades.crear', 'plataforma'),
    ('plataforma.copropiedades.ver', 'plataforma'),
    ('plataforma.cuentas.ver', 'plataforma'),
    ('plataforma.cuentas.bloquear', 'plataforma'),
    ('plataforma.roles.gestionar', 'plataforma'),
    ('plataforma.roles.asignar', 'plataforma'),
    ('plataforma.organizaciones.revisar', 'plataforma'),
    ('plataforma.modulos.gestionar', 'plataforma'),
    ('plataforma.auditoria.ver', 'plataforma'),
    ('plataforma.publicidad.moderar', 'plataforma'),
    ('plataforma.facturacion.ver', 'plataforma'),
    ('copropiedad.datos.ver', 'copropiedad'),
    ('copropiedad.datos.editar', 'copropiedad'),
    ('roles.ver', 'copropiedad'),
    ('roles.gestionar', 'copropiedad'),
    ('roles.asignar', 'copropiedad'),
    ('auditoria.ver', 'copropiedad'),
    ('archivos.subir', 'copropiedad'),
    ('archivos.ver', 'copropiedad'),
    ('archivos.personales.ver', 'copropiedad'),
    ('personas.ver', 'copropiedad'),
    ('personas.editar', 'copropiedad'),
    ('personas.datos_sensibles.ver', 'copropiedad'),
    ('proteccion_datos.gestionar', 'copropiedad'),
    ('organizaciones.crear', 'copropiedad'),
    ('organizaciones.vincular', 'copropiedad'),
    ('contratos.gestionar', 'copropiedad'),
    ('bienes_privados.ver', 'copropiedad'),
    ('bienes_privados.editar', 'copropiedad'),
    ('bienes_privados.coeficientes.editar', 'copropiedad'),
    ('bienes_comunes.ver', 'copropiedad'),
    ('bienes_comunes.editar', 'copropiedad'),
    ('reservas.crear', 'copropiedad'),
    ('reservas.aprobar', 'copropiedad'),
    ('tickets.crear', 'copropiedad'),
    ('tickets.asignar', 'copropiedad'),
    ('tickets.atender', 'copropiedad'),
    ('asambleas.ver', 'copropiedad'),
    ('asambleas.convocar', 'copropiedad'),
    ('asambleas.conducir', 'copropiedad'),
    ('asambleas.votos.ver_detalle', 'copropiedad'),
    ('acceso.registrar', 'copropiedad'),
    ('acceso.visitantes.autorizar', 'copropiedad'),
    ('acceso.bitacora.ver', 'copropiedad'),
    ('contabilidad.ver', 'copropiedad');

-- Roles plantilla (= domain::permisos::ROLES_PLANTILLA). Ids fijos derivados del código.
INSERT INTO roles (id, tenant_id, codigo, nombre, ambito, es_plantilla) VALUES
    (uuid_rol_plantilla('superusuario'), NULL, 'superusuario', 'Superusuario', 'plataforma', TRUE),
    (uuid_rol_plantilla('soporte'), NULL, 'soporte', 'Soporte', 'plataforma', TRUE),
    (uuid_rol_plantilla('comercial'), NULL, 'comercial', 'Comercial', 'plataforma', TRUE),
    (uuid_rol_plantilla('moderador_publicidad'), NULL, 'moderador_publicidad', 'Moderador de publicidad', 'plataforma', TRUE),
    (uuid_rol_plantilla('facturacion'), NULL, 'facturacion', 'Facturación', 'plataforma', TRUE),
    (uuid_rol_plantilla('administrador'), NULL, 'administrador', 'Administrador', 'copropiedad', TRUE),
    (uuid_rol_plantilla('asistente_administrativo'), NULL, 'asistente_administrativo', 'Asistente administrativo', 'copropiedad', TRUE),
    (uuid_rol_plantilla('contador'), NULL, 'contador', 'Contador', 'copropiedad', TRUE),
    (uuid_rol_plantilla('revisor_fiscal'), NULL, 'revisor_fiscal', 'Revisor fiscal', 'copropiedad', TRUE),
    (uuid_rol_plantilla('consejero'), NULL, 'consejero', 'Consejero', 'copropiedad', TRUE),
    (uuid_rol_plantilla('presidente_consejo'), NULL, 'presidente_consejo', 'Presidente del consejo', 'copropiedad', TRUE),
    (uuid_rol_plantilla('comite_convivencia'), NULL, 'comite_convivencia', 'Comité de convivencia', 'copropiedad', TRUE),
    (uuid_rol_plantilla('propietario'), NULL, 'propietario', 'Propietario', 'copropiedad', TRUE),
    (uuid_rol_plantilla('residente'), NULL, 'residente', 'Residente', 'copropiedad', TRUE),
    (uuid_rol_plantilla('vigilante'), NULL, 'vigilante', 'Vigilante / portero', 'copropiedad', TRUE),
    (uuid_rol_plantilla('supervisor_seguridad'), NULL, 'supervisor_seguridad', 'Supervisor de seguridad', 'copropiedad', TRUE),
    (uuid_rol_plantilla('aseo'), NULL, 'aseo', 'Personal de aseo', 'copropiedad', TRUE),
    (uuid_rol_plantilla('mantenimiento'), NULL, 'mantenimiento', 'Mantenimiento', 'copropiedad', TRUE),
    (uuid_rol_plantilla('proveedor_tecnico'), NULL, 'proveedor_tecnico', 'Proveedor técnico', 'copropiedad', TRUE);

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.copropiedades.crear', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.copropiedades.ver', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.cuentas.ver', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.cuentas.bloquear', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.roles.gestionar', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.roles.asignar', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.organizaciones.revisar', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.modulos.gestionar', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.auditoria.ver', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.publicidad.moderar', NULL),
    (uuid_rol_plantilla('superusuario'), 'plataforma', 'plataforma.facturacion.ver', NULL),
    (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.copropiedades.ver', NULL),
    (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.cuentas.ver', NULL),
    (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.cuentas.bloquear', NULL),
    (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.organizaciones.revisar', NULL),
    (uuid_rol_plantilla('soporte'), 'plataforma', 'plataforma.auditoria.ver', NULL),
    (uuid_rol_plantilla('comercial'), 'plataforma', 'plataforma.copropiedades.crear', NULL),
    (uuid_rol_plantilla('comercial'), 'plataforma', 'plataforma.copropiedades.ver', NULL),
    (uuid_rol_plantilla('comercial'), 'plataforma', 'plataforma.modulos.gestionar', NULL),
    (uuid_rol_plantilla('moderador_publicidad'), 'plataforma', 'plataforma.publicidad.moderar', NULL),
    (uuid_rol_plantilla('facturacion'), 'plataforma', 'plataforma.copropiedades.ver', NULL),
    (uuid_rol_plantilla('facturacion'), 'plataforma', 'plataforma.facturacion.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'copropiedad.datos.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'roles.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'roles.gestionar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'roles.asignar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'auditoria.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'archivos.subir', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'archivos.personales.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'personas.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'personas.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'personas.datos_sensibles.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'proteccion_datos.gestionar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'organizaciones.crear', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'organizaciones.vincular', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'contratos.gestionar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_privados.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_privados.coeficientes.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'bienes_comunes.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'reservas.crear', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'reservas.aprobar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'tickets.asignar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'tickets.atender', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'asambleas.convocar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'asambleas.conducir', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'asambleas.votos.ver_detalle', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'acceso.registrar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'acceso.visitantes.autorizar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'acceso.bitacora.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'contabilidad.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'roles.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'archivos.subir', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'personas.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'personas.editar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'organizaciones.crear', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'organizaciones.vincular', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'reservas.aprobar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'tickets.asignar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'acceso.bitacora.ver', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'archivos.subir', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'contabilidad.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'auditoria.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'contabilidad.ver', NULL),
    (uuid_rol_plantilla('revisor_fiscal'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('consejero'), 'copropiedad', 'contabilidad.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'bienes_privados.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'asambleas.convocar', NULL),
    (uuid_rol_plantilla('presidente_consejo'), 'copropiedad', 'contabilidad.ver', NULL),
    (uuid_rol_plantilla('comite_convivencia'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('comite_convivencia'), 'copropiedad', 'tickets.atender', NULL),
    (uuid_rol_plantilla('comite_convivencia'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'reservas.crear', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'asambleas.ver', NULL),
    (uuid_rol_plantilla('propietario'), 'copropiedad', 'acceso.visitantes.autorizar', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'copropiedad.datos.ver', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'archivos.ver', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'reservas.crear', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('residente'), 'copropiedad', 'acceso.visitantes.autorizar', NULL),
    (uuid_rol_plantilla('vigilante'), 'copropiedad', 'acceso.registrar', NULL),
    (uuid_rol_plantilla('vigilante'), 'copropiedad', 'acceso.bitacora.ver', NULL),
    (uuid_rol_plantilla('vigilante'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'acceso.registrar', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'acceso.bitacora.ver', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'acceso.visitantes.autorizar', NULL),
    (uuid_rol_plantilla('supervisor_seguridad'), 'copropiedad', 'tickets.crear', NULL),
    (uuid_rol_plantilla('aseo'), 'copropiedad', 'tickets.atender', NULL),
    (uuid_rol_plantilla('mantenimiento'), 'copropiedad', 'tickets.atender', NULL),
    (uuid_rol_plantilla('mantenimiento'), 'copropiedad', 'bienes_comunes.ver', NULL),
    (uuid_rol_plantilla('proveedor_tecnico'), 'copropiedad', 'tickets.atender', NULL);
