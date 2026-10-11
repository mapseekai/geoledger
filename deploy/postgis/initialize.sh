#!/bin/sh
# Runs once in a fresh PostgreSQL volume under the initialization administrator.
set -eu
psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" --set=ON_ERROR_STOP=1 --set=app_password="$GL_DB_APP_PASSWORD" <<'SQL'
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE ROLE geoledger_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION PASSWORD :'app_password';
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE, CREATE ON SCHEMA public TO geoledger_app;
SQL
