-- Revierte el renombre `inmuebles` → `bienes_privados` (orden inverso al .up.sql).

CREATE OR REPLACE FUNCTION validar_voto() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_tipo TEXT;
BEGIN
    SELECT tipo_voto INTO v_tipo
    FROM preguntas
    WHERE id = NEW.pregunta_id AND tenant_id = NEW.tenant_id;

    IF v_tipo = 'secreto' AND NEW.inmueble_id IS NOT NULL THEN
        RAISE EXCEPTION 'un voto secreto no puede referenciar inmueble ni participación'
            USING ERRCODE = 'check_violation';
    ELSIF v_tipo = 'nominal' AND NEW.inmueble_id IS NULL THEN
        RAISE EXCEPTION 'un voto nominal debe referenciar inmueble y participación'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT conrelid::regclass AS tabla, conname
        FROM pg_constraint
        WHERE conrelid IN ('bienes_privados'::regclass, 'inscripciones'::regclass,
                           'participaciones'::regclass, 'votos'::regclass)
          AND conname LIKE '%bien%privado%'
    LOOP
        EXECUTE format('ALTER TABLE %s RENAME CONSTRAINT %I TO %I', r.tabla, r.conname,
            replace(replace(r.conname, 'bienes_privados', 'inmuebles'), 'bien_privado', 'inmueble'));
    END LOOP;
END
$$;

ALTER TABLE votos           RENAME COLUMN bien_privado_id TO inmueble_id;
ALTER TABLE participaciones RENAME COLUMN bien_privado_id TO inmueble_id;
ALTER TABLE inscripciones   RENAME COLUMN bien_privado_id TO inmueble_id;

ALTER TABLE bienes_privados RENAME TO inmuebles;
