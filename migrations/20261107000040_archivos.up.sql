-- Archivos y adjuntos por copropiedad (plan 01 §3, ADR 0003).
-- Los bytes viven en un almacén (disco del VPS o S3); aquí solo la clave lógica y el almacén
-- donde están hoy. Nunca rutas absolutas del servidor: migrar de almacén = copiar claves y
-- cambiar `almacen`.
CREATE TABLE archivos (
    id               UUID        PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES copropiedades (id),
    -- `{tenant}/{aa}/{bb}/{id}` (domain::ClaveArchivo).
    clave            TEXT        NOT NULL UNIQUE
                     CHECK (clave ~ '^[0-9a-f-]{36}/[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f-]{36}$'),
    almacen          TEXT        NOT NULL CHECK (almacen IN ('local', 's3')),
    nombre_original  TEXT        NOT NULL CHECK (char_length(nombre_original) BETWEEN 1 AND 255),
    -- Detectado por el contenido (magic bytes), no por la extensión.
    tipo_mime        TEXT        NOT NULL CHECK (tipo_mime IN (
                         'application/pdf', 'image/jpeg', 'image/png', 'image/webp', 'image/heic')),
    tamano_bytes     BIGINT      NOT NULL CHECK (tamano_bytes > 0),
    sha256           TEXT        NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    clasificacion    TEXT        NOT NULL CHECK (clasificacion IN (
                         'publico', 'interno', 'personal', 'sensible')),
    -- Actor del contexto (como auditoria.actor_id); sin FK hasta que el plan 02 defina
    -- cuentas y personas. NULL = sistema.
    subido_por       UUID,
    creado_en        TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, id),
    -- La clave empieza por el tenant: un archivo no puede quedar bajo la carpeta de otra
    -- copropiedad.
    CHECK (starts_with(clave, tenant_id::text || '/'))
);

-- Migración entre almacenes: archivos pendientes de un almacén, por copropiedad.
CREATE INDEX ix_archivos_almacen ON archivos (tenant_id, almacen, id);

ALTER TABLE archivos ENABLE ROW LEVEL SECURITY;
CREATE POLICY aislamiento_tenant ON archivos
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());

-- Sin DELETE: el borrado de archivos llegará con las reglas de retención (plan de protección
-- de datos). UPDATE solo para cambiar de almacén.
GRANT SELECT, INSERT ON archivos TO dad_app;
GRANT UPDATE (almacen) ON archivos TO dad_app;
