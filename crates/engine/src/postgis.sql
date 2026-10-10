CREATE TABLE gl_purge (project text PRIMARY KEY);
CREATE FUNCTION gl_immutable() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF TG_OP='DELETE' AND TG_TABLE_NAME<>'gl_audit_events' AND EXISTS(SELECT 1 FROM gl_purge WHERE project=OLD.project) THEN RETURN OLD; END IF; RAISE EXCEPTION 'immutable version record'; END $$;
CREATE FUNCTION gl_json_field(value text, key text) RETURNS text LANGUAGE sql IMMUTABLE STRICT AS $$ SELECT value::jsonb ->> key $$;
CREATE TABLE gl_format (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), version integer NOT NULL CHECK(version=11));
INSERT INTO gl_format VALUES(true,11);
CREATE TABLE gl_projects (
 id text PRIMARY KEY, name text NOT NULL, head bigint NOT NULL DEFAULT 0 CHECK(head>=0),
 state text NOT NULL DEFAULT 'active' CHECK(state IN ('active','archived','deleted')));
CREATE TABLE gl_project_members (
 project text REFERENCES gl_projects(id), subject text NOT NULL,
 role text NOT NULL CHECK(role IN ('owner','editor','viewer')), removed boolean NOT NULL DEFAULT false,
 PRIMARY KEY(project,subject));
CREATE INDEX gl_project_members_subject ON gl_project_members(subject,project);
CREATE TABLE gl_datasets (
 project text REFERENCES gl_projects(id), id text NOT NULL, name text NOT NULL,
 geometry_type text NOT NULL CHECK(geometry_type IN ('point','line','polygon')),
 coordinate_dimension bigint NOT NULL DEFAULT 2 CHECK(coordinate_dimension IN (2,3)),
 postgis_source text,
 PRIMARY KEY(project,id), UNIQUE(project,name));
CREATE TABLE gl_workspaces (
 project text NOT NULL, id text NOT NULL, owner text NOT NULL, base_revision bigint NOT NULL CHECK(base_revision>=0),
 version bigint NOT NULL DEFAULT 0 CHECK(version>=0), status text NOT NULL DEFAULT 'open' CHECK(status IN ('open','published','discarded')),
 PRIMARY KEY(project,id), FOREIGN KEY(project,owner) REFERENCES gl_project_members(project,subject));
CREATE TABLE gl_workspace_changes (
 project text NOT NULL, workspace text NOT NULL, dataset text NOT NULL, feature_id text COLLATE "C" NOT NULL,
 resolved_head bigint CHECK(resolved_head IS NULL OR resolved_head >= 0), resolution_stale boolean NOT NULL DEFAULT false, properties text, geometry_json text, geom geometry CHECK(geom IS NULL OR ST_SRID(geom)=4326),
 PRIMARY KEY(project,workspace,dataset,feature_id),
 FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 CHECK(properties IS NULL OR jsonb_typeof(properties::jsonb)='object'), CHECK(properties IS NOT NULL OR geometry_json IS NULL));
CREATE INDEX workspace_active_resolutions ON gl_workspace_changes(project,workspace) WHERE resolved_head IS NOT NULL AND NOT resolution_stale;
CREATE INDEX workspace_geom ON gl_workspace_changes USING gist(geom);
CREATE TABLE gl_commits (
 project text NOT NULL REFERENCES gl_projects(id), revision bigint NOT NULL CHECK(revision>0),
 workspace text NOT NULL, subject text NOT NULL, message text NOT NULL, created_at text NOT NULL DEFAULT to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'),
 PRIMARY KEY(project,revision), FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject), FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id));
CREATE TABLE gl_history (
 project text NOT NULL, dataset text NOT NULL, feature_id text COLLATE "C" NOT NULL, valid_from bigint NOT NULL, valid_to bigint,
 properties text, geometry_json text, geom geometry CHECK(geom IS NULL OR ST_SRID(geom)=4326),
 PRIMARY KEY(project,dataset,feature_id,valid_from),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 FOREIGN KEY(project,valid_from) REFERENCES gl_commits(project,revision),
 FOREIGN KEY(project,valid_to) REFERENCES gl_commits(project,revision),
 CHECK(valid_to IS NULL OR valid_to>valid_from),
 CHECK(properties IS NULL OR jsonb_typeof(properties::jsonb)='object'), CHECK(properties IS NOT NULL OR geometry_json IS NULL));
CREATE UNIQUE INDEX history_live ON gl_history(project,dataset,feature_id) WHERE valid_to IS NULL;
CREATE INDEX history_geom ON gl_history USING gist(geom);
CREATE TABLE gl_commit_changes (
 project text NOT NULL, revision bigint NOT NULL, dataset text NOT NULL, feature_id text COLLATE "C" NOT NULL,
 before_value text, after_value text, PRIMARY KEY(project,revision,dataset,feature_id),
 FOREIGN KEY(project,revision) REFERENCES gl_commits(project,revision),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id));
CREATE TABLE gl_idempotency (
 project text NOT NULL, subject text NOT NULL, request_id text NOT NULL, payload text NOT NULL, result text NOT NULL,
 PRIMARY KEY(project,subject,request_id), FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject));
CREATE TABLE gl_audit_events (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, project text NOT NULL REFERENCES gl_projects(id),
 subject text NOT NULL, action text NOT NULL, detail text NOT NULL, created_at text NOT NULL DEFAULT to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'),
 FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject));
CREATE INDEX audit_project_id ON gl_audit_events(project,id);
CREATE TRIGGER gl_commits_immutable BEFORE UPDATE OR DELETE ON gl_commits FOR EACH ROW EXECUTE FUNCTION gl_immutable();
CREATE TRIGGER gl_commit_changes_immutable BEFORE UPDATE OR DELETE ON gl_commit_changes FOR EACH ROW EXECUTE FUNCTION gl_immutable();
CREATE TRIGGER gl_audit_events_immutable BEFORE UPDATE OR DELETE ON gl_audit_events FOR EACH ROW EXECUTE FUNCTION gl_immutable();
CREATE TRIGGER gl_idempotency_immutable BEFORE UPDATE OR DELETE ON gl_idempotency FOR EACH ROW EXECUTE FUNCTION gl_immutable();
CREATE FUNCTION gl_close_history() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
IF TG_OP='DELETE' AND EXISTS(SELECT 1 FROM gl_purge WHERE project=OLD.project) THEN RETURN OLD; END IF;
IF TG_OP='UPDATE' THEN
 IF OLD.valid_to IS NULL AND NEW.valid_to IS NOT NULL AND (to_jsonb(NEW)-'valid_to')=(to_jsonb(OLD)-'valid_to') AND ST_AsEWKB(NEW.geom) IS NOT DISTINCT FROM ST_AsEWKB(OLD.geom) THEN RETURN NEW; END IF;
END IF; RAISE EXCEPTION 'immutable history'; END $$;
CREATE TRIGGER gl_history_immutable BEFORE UPDATE OR DELETE ON gl_history FOR EACH ROW EXECUTE FUNCTION gl_close_history();

CREATE UNIQUE INDEX dataset_source_table ON gl_datasets ((postgis_source::jsonb->'table'->>'schema'),(postgis_source::jsonb->'table'->>'table')) WHERE postgis_source IS NOT NULL;

CREATE FUNCTION gl_source_guard() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF NOT coalesce(TG_RELID::text = ANY(string_to_array(current_setting('geoledger.publication_table',true), ',')),false) THEN
  RAISE EXCEPTION 'write this versioned table through GeoLedger';
 END IF;
 RETURN NULL;
END $$;

-- A transactional candidate set covers indirect business-trigger writes as well as API edits.
CREATE TABLE gl_source_state (
 schema_name text NOT NULL, table_name text NOT NULL, stamp text NOT NULL, definition text NOT NULL, binding text NOT NULL,
 dirty_all boolean NOT NULL DEFAULT false, PRIMARY KEY(schema_name,table_name));
CREATE TABLE gl_source_changes (
 schema_name text NOT NULL, table_name text NOT NULL, feature_id text COLLATE "C" NOT NULL,
 PRIMARY KEY(schema_name,table_name,feature_id),
 FOREIGN KEY(schema_name,table_name) REFERENCES gl_source_state ON DELETE CASCADE);
CREATE FUNCTION gl_source_track() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP='TRUNCATE' THEN
  UPDATE public.gl_source_state SET dirty_all=true WHERE schema_name=TG_TABLE_SCHEMA AND table_name=TG_TABLE_NAME;
 ELSE
  IF TG_OP IN ('UPDATE','DELETE') THEN
   INSERT INTO public.gl_source_changes VALUES(TG_TABLE_SCHEMA,TG_TABLE_NAME,to_jsonb(OLD)->>TG_ARGV[0]) ON CONFLICT DO NOTHING;
  END IF;
  IF TG_OP IN ('INSERT','UPDATE') THEN
   INSERT INTO public.gl_source_changes VALUES(TG_TABLE_SCHEMA,TG_TABLE_NAME,to_jsonb(NEW)->>TG_ARGV[0]) ON CONFLICT DO NOTHING;
  END IF;
 END IF;
 RETURN NULL;
END $$;
