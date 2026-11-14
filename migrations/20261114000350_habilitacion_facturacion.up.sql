-- Habilitación de una entidad ante la DIAN con el motor propio (plan 18 parte B, plan 17 fase 5).
--
-- Cada entidad que factura con el software propio registra en la DIAN su SoftwareID, su PIN y el
-- TestSetId del set de pruebas; carga su certificado de firma (.p12); envía el set de pruebas;
-- y, aprobado, pasa a producción. Los secretos (el .p12, su contraseña y el PIN) se guardan
-- cifrados con la clave maestra del servidor (AES-256-GCM, atados a la entidad): nunca en claro,
-- en logs ni en la auditoría.

CREATE TABLE certificados_firma (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES entidades (id),
    -- El .p12 cifrado (unos KB): junto a su contraseña, nunca en el almacén de archivos.
    p12_cifrado         BYTEA       NOT NULL CHECK (octet_length(p12_cifrado) <= 65536),
    contrasena_cifrada  BYTEA       NOT NULL,
    sujeto              TEXT        NOT NULL CHECK (char_length(sujeto) BETWEEN 1 AND 1000),
    emisor              TEXT        NOT NULL CHECK (char_length(emisor) BETWEEN 1 AND 1000),
    serial              TEXT        NOT NULL CHECK (serial ~ '^[0-9]{1,80}$'),
    vigente_desde       TIMESTAMPTZ NOT NULL,
    vigente_hasta       TIMESTAMPTZ NOT NULL CHECK (vigente_hasta > vigente_desde),
    activo              BOOLEAN     NOT NULL DEFAULT TRUE,
    cargado_por         UUID,
    cargado_en          TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id)
);
-- Un solo certificado activo por entidad: cargar otro desactiva el anterior.
CREATE UNIQUE INDEX ux_certificado_activo ON certificados_firma (tenant_id) WHERE activo;
COMMENT ON COLUMN certificados_firma.p12_cifrado IS
    'secreto: el .p12 (llave privada de firma) cifrado con la clave maestra';
COMMENT ON COLUMN certificados_firma.contrasena_cifrada IS
    'secreto: contraseña del .p12 cifrada con la clave maestra; nunca en la API ni en los logs';

CREATE TABLE configuraciones_facturacion (
    tenant_id        UUID        PRIMARY KEY REFERENCES entidades (id),
    software_id      TEXT        NOT NULL CHECK (software_id ~ '^[0-9A-Za-z-]{1,100}$'),
    pin_cifrado      BYTEA       NOT NULL,
    test_set_id      TEXT        CHECK (test_set_id ~ '^[0-9A-Za-z-]{1,100}$'),
    ambiente         TEXT        NOT NULL DEFAULT 'habilitacion'
                     CHECK (ambiente IN ('habilitacion', 'produccion')),
    estado           TEXT        NOT NULL DEFAULT 'registrado' CHECK (estado IN (
                         'registrado', 'en_pruebas', 'habilitado', 'produccion')),
    certificado_id   UUID,
    actualizado_por  UUID,
    actualizado_en   TIMESTAMPTZ NOT NULL,
    CHECK ((estado = 'produccion') = (ambiente = 'produccion')),
    FOREIGN KEY (tenant_id, certificado_id) REFERENCES certificados_firma (tenant_id, id)
);
COMMENT ON COLUMN configuraciones_facturacion.pin_cifrado IS
    'secreto: PIN del software cifrado con la clave maestra; nunca en la API ni en los logs';

-- Documentos del set de pruebas enviados a la DIAN (ambiente de habilitación).
CREATE TABLE envios_set_pruebas (
    id             UUID        PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES entidades (id),
    tipo           TEXT        NOT NULL CHECK (tipo IN ('factura', 'nota_credito')),
    prefijo        TEXT        NOT NULL CHECK (prefijo ~ '^[A-Z0-9]{0,4}$'),
    numero         BIGINT      NOT NULL CHECK (numero > 0),
    codigo         TEXT        NOT NULL CHECK (char_length(codigo) BETWEEN 10 AND 200),
    clave          TEXT        CHECK (char_length(clave) BETWEEN 1 AND 200),
    estado         TEXT        NOT NULL CHECK (estado IN ('enviado', 'aceptado', 'rechazado',
                                                          'sin_respuesta')),
    respuesta      TEXT        CHECK (char_length(respuesta) <= 4000),
    enviado_en     TIMESTAMPTZ NOT NULL,
    consultado_en  TIMESTAMPTZ,
    UNIQUE (tenant_id, prefijo, numero)
);
CREATE INDEX ix_envios_set_pruebas ON envios_set_pruebas (tenant_id, enviado_en);

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['certificados_firma', 'configuraciones_facturacion',
                             'envios_set_pruebas'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END $$;
GRANT SELECT, INSERT ON certificados_firma TO dad_app;
GRANT UPDATE (activo) ON certificados_firma TO dad_app;
GRANT SELECT, INSERT, UPDATE ON configuraciones_facturacion, envios_set_pruebas TO dad_app;

-- Consecutivo anual de archivos enviados a la DIAN (nombre del XML y del ZIP, anexo §6.5.7).
ALTER TABLE consecutivos DROP CONSTRAINT consecutivos_serie_check;
ALTER TABLE consecutivos ADD CONSTRAINT consecutivos_serie_check
    CHECK (serie IN ('ticket', 'orden_trabajo', 'factura_venta', 'archivo_dian')
           OR serie ~ '^comprobante:[A-Z][A-Z0-9]{1,4}$');

-- Permisos: configurar la habilitación y gestionar el certificado (el segundo, solo el
-- administrador: el certificado firma en nombre de la entidad).
INSERT INTO permisos (codigo, ambito) VALUES
    ('facturacion.configurar', 'copropiedad'),
    ('facturacion.certificado.gestionar', 'copropiedad');
INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'facturacion.configurar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'facturacion.certificado.gestionar', NULL),
    (uuid_rol_plantilla('contador'), 'copropiedad', 'facturacion.configurar', NULL);
