-- =====================================================================
-- Terminología de la Ley 675 de 2001 (art. 3): la unidad con coeficiente es un
-- "bien privado o de dominio particular". Se renombra `inmuebles` → `bienes_privados` y las
-- columnas `inmueble_id` → `bien_privado_id`. Ver docs/adr/0002-terminologia-ley-675.md.
--
-- Migración nueva (no se edita `esquema_inicial`): solo renombra; no cambia datos, permisos,
-- políticas RLS ni restricciones (PostgreSQL los conserva al renombrar).
-- =====================================================================

ALTER TABLE inmuebles RENAME TO bienes_privados;

ALTER TABLE inscripciones   RENAME COLUMN inmueble_id TO bien_privado_id;
ALTER TABLE participaciones RENAME COLUMN inmueble_id TO bien_privado_id;
ALTER TABLE votos           RENAME COLUMN inmueble_id TO bien_privado_id;

-- Nombres de restricciones (y de sus índices) generados a partir de los nombres viejos.
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT conrelid::regclass AS tabla, conname
        FROM pg_constraint
        WHERE conrelid IN ('bienes_privados'::regclass, 'inscripciones'::regclass,
                           'participaciones'::regclass, 'votos'::regclass)
          AND conname LIKE '%inmueble%'
    LOOP
        EXECUTE format('ALTER TABLE %s RENAME CONSTRAINT %I TO %I', r.tabla, r.conname,
            replace(replace(r.conname, 'inmuebles', 'bienes_privados'), 'inmueble', 'bien_privado'));
    END LOOP;
END
$$;

-- El cuerpo de las funciones PL/pgSQL es texto: hay que reescribirlo con la columna nueva.
CREATE OR REPLACE FUNCTION validar_voto() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_tipo TEXT;
BEGIN
    SELECT tipo_voto INTO v_tipo
    FROM preguntas
    WHERE id = NEW.pregunta_id AND tenant_id = NEW.tenant_id;

    IF v_tipo = 'secreto' AND NEW.bien_privado_id IS NOT NULL THEN
        RAISE EXCEPTION 'un voto secreto no puede referenciar bien privado ni participación'
            USING ERRCODE = 'check_violation';
    ELSIF v_tipo = 'nominal' AND NEW.bien_privado_id IS NULL THEN
        RAISE EXCEPTION 'un voto nominal debe referenciar bien privado y participación'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
