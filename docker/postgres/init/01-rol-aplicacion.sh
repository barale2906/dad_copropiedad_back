#!/bin/sh
# Solo desarrollo: crea el rol con LOGIN que usa la aplicación (APP_DATABASE_URL).
# Se ejecuta una única vez, al crear el volumen de datos. La migración inicial también
# garantiza que el rol exista (NOLOGIN) y le otorga los permisos mínimos.
# En producción el rol y su contraseña los gestiona operaciones, no este script.
set -eu

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<EOSQL
DO \$\$
BEGIN
    CREATE ROLE dad_app LOGIN PASSWORD '${APP_DB_PASSWORD}'
        NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
EXCEPTION WHEN duplicate_object THEN
    ALTER ROLE dad_app LOGIN PASSWORD '${APP_DB_PASSWORD}';
END
\$\$;
EOSQL
