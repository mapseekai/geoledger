-- Persistent v1 schema and trigger identities are retained for existing repositories.
CREATE SCHEMA IF NOT EXISTS _spatial_version;
CREATE TABLE IF NOT EXISTS _spatial_version.format(version integer PRIMARY KEY CHECK(version=1));
INSERT INTO _spatial_version.format VALUES(1) ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS _spatial_version.repositories (
    id text PRIMARY KEY, head text NOT NULL, operation text
);
CREATE TABLE IF NOT EXISTS _spatial_version.tracked (
    table_oid oid PRIMARY KEY,
    repository_id text NOT NULL REFERENCES _spatial_version.repositories(id),
    dataset text NOT NULL,
    UNIQUE(repository_id, dataset)
);
CREATE TABLE IF NOT EXISTS _spatial_version.dirty (
    repository_id text NOT NULL, dataset text NOT NULL, pk text NOT NULL,
    PRIMARY KEY(repository_id, dataset, pk)
);

CREATE INDEX IF NOT EXISTS dirty_pk_c_v1 ON _spatial_version.dirty(repository_id,dataset,pk COLLATE "C");

CREATE OR REPLACE FUNCTION _spatial_version.track_row_v1() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, _spatial_version AS $$
BEGIN
    IF TG_OP <> 'INSERT' THEN
        INSERT INTO _spatial_version.dirty(repository_id,dataset,pk)
        VALUES(TG_ARGV[0],TG_ARGV[1],to_jsonb(OLD)->>TG_ARGV[2])
        ON CONFLICT DO NOTHING;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        INSERT INTO _spatial_version.dirty(repository_id,dataset,pk)
        VALUES(TG_ARGV[0],TG_ARGV[1],to_jsonb(NEW)->>TG_ARGV[2])
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NULL; -- AFTER trigger; both OLD and NEW PKs are retained on UPDATE.
END;
$$;

CREATE OR REPLACE FUNCTION _spatial_version.reject_truncate_v1() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION USING ERRCODE='0A000',
        MESSAGE='TRUNCATE is disabled for version-controlled tables; use DELETE or restore';
END;
$$;
