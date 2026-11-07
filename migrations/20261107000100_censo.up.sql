-- Libros del censo (plan 07a, ADR 0007): residencias, vehículos, mascotas y propuestas de cambio
-- por autoservicio. El libro de propietarios es una consulta sobre `titularidades` (plan 03).
--
-- Garantías de la base de datos:
--   1. Una persona reside una sola vez a la vez en un bien privado, y cada bien tiene a lo sumo
--      un responsable vigente ante la copropiedad (índices únicos parciales).
--   2. Una placa está vigente en un solo vehículo por copropiedad.
--   3. Las propuestas de cambio solo se resuelven una vez (de `pendiente` a `aprobado` o
--      `rechazado`).

-- ---------------------------------------------------------------------
-- Personas: documento, menores, contacto de emergencia y movilidad reducida
-- ---------------------------------------------------------------------
ALTER TABLE personas
    ADD COLUMN tipo_documento               TEXT CHECK (tipo_documento IN (
                                                'CC', 'CE', 'TI', 'RC', 'PA', 'PPT', 'NIT')),
    ADD COLUMN fecha_nacimiento             DATE,
    ADD COLUMN contacto_emergencia_nombre   TEXT CHECK (char_length(contacto_emergencia_nombre) <= 120),
    ADD COLUMN contacto_emergencia_telefono TEXT CHECK (contacto_emergencia_telefono ~ '^[1-9][0-9]{7,14}$'),
    -- Sirve para evacuación y parqueaderos; solo con consentimiento del titular.
    ADD COLUMN movilidad_reducida           BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN actualizado_en               TIMESTAMPTZ NOT NULL DEFAULT now();
COMMENT ON COLUMN personas.fecha_nacimiento IS 'dato_sensible: identifica a menores de edad (Ley 1581, art. 7)';
COMMENT ON COLUMN personas.contacto_emergencia_nombre IS 'dato_personal: nombre de un tercero';
COMMENT ON COLUMN personas.contacto_emergencia_telefono IS 'dato_personal: teléfono de un tercero';
COMMENT ON COLUMN personas.movilidad_reducida IS 'dato_sensible: condición de salud';
CREATE INDEX ix_personas_cuenta ON personas (tenant_id, cuenta_id) WHERE cuenta_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Residencias
-- ---------------------------------------------------------------------
CREATE TABLE residencias (
    id                   UUID        PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id      UUID        NOT NULL,
    persona_id           UUID        NOT NULL,
    calidad              TEXT        NOT NULL CHECK (calidad IN (
                             'propietario_residente', 'tenedor', 'familiar', 'huesped_temporal',
                             'empleado_domestico')),
    -- Cabeza del bien privado ante la copropiedad.
    es_responsable       BOOLEAN     NOT NULL DEFAULT FALSE,
    -- Propietario que autoriza al tenedor (arrendatario u otro título).
    autorizado_por       UUID,
    contrato_archivo_id  UUID,
    vigente_desde        TIMESTAMPTZ NOT NULL,
    vigente_hasta        TIMESTAMPTZ CHECK (vigente_hasta >= vigente_desde),
    creado_en            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    CHECK (calidad <> 'tenedor' OR autorizado_por IS NOT NULL),
    FOREIGN KEY (tenant_id, bien_privado_id)     REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)          REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, autorizado_por)      REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, contrato_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_residencia_vigente ON residencias (bien_privado_id, persona_id)
    WHERE vigente_hasta IS NULL;
CREATE UNIQUE INDEX ux_responsable_vigente ON residencias (bien_privado_id)
    WHERE vigente_hasta IS NULL AND es_responsable;
CREATE INDEX ix_residencias_bien ON residencias (tenant_id, bien_privado_id, vigente_desde);
CREATE INDEX ix_residencias_persona ON residencias (tenant_id, persona_id);

-- ---------------------------------------------------------------------
-- Vehículos
-- ---------------------------------------------------------------------
CREATE TABLE vehiculos (
    id                  UUID        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id     UUID        NOT NULL,
    persona_id          UUID,
    tipo                TEXT        NOT NULL CHECK (tipo IN ('carro', 'moto', 'bicicleta', 'otro')),
    -- Normalizada: mayúsculas, sin espacios ni guiones (domain::Placa).
    placa               TEXT        CHECK (placa ~ '^[A-Z0-9]{4,8}$'),
    marca               TEXT        CHECK (char_length(marca) <= 60),
    color               TEXT        CHECK (char_length(color) <= 40),
    tarjeta_archivo_id  UUID,
    vigente_desde       TIMESTAMPTZ NOT NULL,
    vigente_hasta       TIMESTAMPTZ CHECK (vigente_hasta >= vigente_desde),
    UNIQUE (tenant_id, id),
    CHECK (tipo IN ('bicicleta', 'otro') OR placa IS NOT NULL),
    FOREIGN KEY (tenant_id, bien_privado_id)    REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, persona_id)         REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, tarjeta_archivo_id) REFERENCES archivos (tenant_id, id)
);
CREATE UNIQUE INDEX ux_placa_vigente ON vehiculos (tenant_id, placa)
    WHERE vigente_hasta IS NULL AND placa IS NOT NULL;
CREATE INDEX ix_vehiculos_bien ON vehiculos (tenant_id, bien_privado_id);
COMMENT ON COLUMN vehiculos.placa IS 'dato_personal: identifica al dueño del vehículo';

-- ---------------------------------------------------------------------
-- Mascotas (Ley 1801 de 2016, Ley 746 de 2002)
-- ---------------------------------------------------------------------
CREATE TABLE mascotas (
    id                        UUID        PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id           UUID        NOT NULL,
    responsable_id            UUID,
    especie                   TEXT        NOT NULL CHECK (especie IN ('perro', 'gato', 'otro')),
    nombre                    TEXT        NOT NULL CHECK (char_length(nombre) BETWEEN 1 AND 60),
    raza                      TEXT        CHECK (char_length(raza) <= 60),
    potencialmente_peligrosa  BOOLEAN     NOT NULL DEFAULT FALSE,
    poliza_numero             TEXT        CHECK (char_length(poliza_numero) <= 60),
    poliza_vence              DATE,
    poliza_archivo_id         UUID,
    vacunas_vencen            DATE,
    carne_archivo_id          UUID,
    esterilizada              BOOLEAN     NOT NULL DEFAULT FALSE,
    foto_archivo_id           UUID,
    estado                    TEXT        NOT NULL DEFAULT 'activa'
                              CHECK (estado IN ('activa', 'fallecida', 'trasladada')),
    creado_en                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    actualizado_en            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, bien_privado_id)   REFERENCES bienes_privados (tenant_id, id),
    FOREIGN KEY (tenant_id, responsable_id)    REFERENCES personas (tenant_id, id),
    FOREIGN KEY (tenant_id, poliza_archivo_id) REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, carne_archivo_id)  REFERENCES archivos (tenant_id, id),
    FOREIGN KEY (tenant_id, foto_archivo_id)   REFERENCES archivos (tenant_id, id)
);
CREATE INDEX ix_mascotas_bien ON mascotas (tenant_id, bien_privado_id);
-- Alertas de vencimientos (vacunas, pólizas) de las mascotas activas.
CREATE INDEX ix_mascotas_vencimientos ON mascotas (tenant_id, vacunas_vencen, poliza_vence)
    WHERE estado = 'activa';

-- ---------------------------------------------------------------------
-- Propuestas de cambio del censo (autoservicio de residentes y propietarios)
-- ---------------------------------------------------------------------
CREATE TABLE cambios_censo (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    bien_privado_id  UUID        NOT NULL,
    tipo             TEXT        NOT NULL CHECK (tipo IN ('residencia', 'vehiculo', 'mascota')),
    datos            JSONB       NOT NULL,
    estado           TEXT        NOT NULL DEFAULT 'pendiente'
                     CHECK (estado IN ('pendiente', 'aprobado', 'rechazado')),
    propuesto_por    UUID        NOT NULL,   -- cuenta
    propuesto_en     TIMESTAMPTZ NOT NULL,
    revisado_por     UUID,
    revisado_en      TIMESTAMPTZ,
    motivo           TEXT        CHECK (char_length(motivo) <= 500),
    -- Residencia, vehículo o mascota creados al aprobar.
    resultado_id     UUID,
    UNIQUE (tenant_id, id),
    CHECK ((estado = 'pendiente') = (revisado_en IS NULL)),
    FOREIGN KEY (tenant_id, bien_privado_id) REFERENCES bienes_privados (tenant_id, id)
);
CREATE INDEX ix_cambios_censo_pendientes ON cambios_censo (tenant_id, estado, propuesto_en);
CREATE INDEX ix_cambios_censo_cuenta ON cambios_censo (tenant_id, propuesto_por);
COMMENT ON COLUMN cambios_censo.datos IS 'dato_personal: datos propuestos de personas, vehículos o mascotas';

-- Solo se resuelve una vez.
CREATE FUNCTION exigir_cambio_pendiente() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.estado <> 'pendiente' THEN
        RAISE EXCEPTION 'el cambio del censo ya fue %', OLD.estado USING ERRCODE = 'V0006';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_cambios_censo_una_vez BEFORE UPDATE ON cambios_censo
    FOR EACH ROW EXECUTE FUNCTION exigir_cambio_pendiente();

-- ---------------------------------------------------------------------
-- RLS y permisos
-- ---------------------------------------------------------------------
DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['residencias', 'vehiculos', 'mascotas', 'cambios_censo'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY aislamiento_tenant ON %I '
            'USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id())', t);
    END LOOP;
END
$$;

GRANT SELECT, INSERT ON residencias, vehiculos, mascotas, cambios_censo TO dad_app;
GRANT UPDATE (vigente_hasta, es_responsable) ON residencias TO dad_app;
GRANT UPDATE (vigente_hasta, persona_id, marca, color) ON vehiculos TO dad_app;
GRANT UPDATE (responsable_id, raza, potencialmente_peligrosa, poliza_numero, poliza_vence,
              poliza_archivo_id, vacunas_vencen, carne_archivo_id, esterilizada,
              foto_archivo_id, estado, actualizado_en) ON mascotas TO dad_app;
GRANT UPDATE (estado, revisado_por, revisado_en, motivo, resultado_id) ON cambios_censo TO dad_app;

-- Catálogo de permisos (= domain::Permiso) y plantillas.
INSERT INTO permisos (codigo, ambito) VALUES
    ('censo.ver', 'copropiedad'),
    ('censo.editar', 'copropiedad'),
    ('censo.aprobar', 'copropiedad'),
    ('censo.exportar', 'copropiedad');

INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id) VALUES
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'censo.ver', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'censo.editar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'censo.aprobar', NULL),
    (uuid_rol_plantilla('administrador'), 'copropiedad', 'censo.exportar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'censo.ver', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'censo.editar', NULL),
    (uuid_rol_plantilla('asistente_administrativo'), 'copropiedad', 'censo.aprobar', NULL);
