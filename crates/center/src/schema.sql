CREATE SCHEMA _geoledger_center;
CREATE TABLE _geoledger_center.format (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), version integer NOT NULL CHECK(version=2));
INSERT INTO _geoledger_center.format VALUES(true,2);
CREATE TABLE _geoledger_center.projects (
 id uuid PRIMARY KEY, name text NOT NULL, head bigint NOT NULL DEFAULT 0 CHECK(head>=0));
CREATE TABLE _geoledger_center.project_members (
 project uuid REFERENCES _geoledger_center.projects(id), subject text NOT NULL,
 role text NOT NULL CHECK(role IN ('owner','editor','viewer')), PRIMARY KEY(project,subject));
CREATE TABLE _geoledger_center.datasets (
 project uuid REFERENCES _geoledger_center.projects(id), id uuid NOT NULL, name text NOT NULL,
 PRIMARY KEY(project,id), UNIQUE(project,name));
CREATE TABLE _geoledger_center.workspaces (
 project uuid NOT NULL, id uuid NOT NULL, owner text NOT NULL, base_revision bigint NOT NULL CHECK(base_revision>=0),
 version bigint NOT NULL DEFAULT 0 CHECK(version>=0), status text NOT NULL DEFAULT 'open' CHECK(status IN ('open','published','discarded')),
 PRIMARY KEY(project,id), FOREIGN KEY(project,owner) REFERENCES _geoledger_center.project_members(project,subject));
CREATE TABLE _geoledger_center.workspace_changes (
 project uuid NOT NULL, workspace uuid NOT NULL, dataset uuid NOT NULL, feature_id text COLLATE "C" NOT NULL,
 resolved_head bigint CHECK(resolved_head IS NULL OR resolved_head >= 0), resolution_stale boolean NOT NULL DEFAULT false, properties jsonb, geom geometry CHECK(geom IS NULL OR (ST_SRID(geom)=4326 AND ST_NDims(geom) IN (2,3))),
 PRIMARY KEY(project,workspace,dataset,feature_id),
 FOREIGN KEY(project,workspace) REFERENCES _geoledger_center.workspaces(project,id),
 FOREIGN KEY(project,dataset) REFERENCES _geoledger_center.datasets(project,id),
 CHECK(properties IS NULL OR jsonb_typeof(properties)='object'), CHECK(properties IS NOT NULL OR geom IS NULL));
CREATE INDEX workspace_geom ON _geoledger_center.workspace_changes USING gist(geom);
CREATE TABLE _geoledger_center.commits (
 project uuid NOT NULL REFERENCES _geoledger_center.projects(id), revision bigint NOT NULL CHECK(revision>0),
 workspace uuid NOT NULL, subject text NOT NULL, message text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(project,revision), FOREIGN KEY(project,subject) REFERENCES _geoledger_center.project_members(project,subject), FOREIGN KEY(project,workspace) REFERENCES _geoledger_center.workspaces(project,id));
CREATE TABLE _geoledger_center.features (
 project uuid NOT NULL, dataset uuid NOT NULL, feature_id text COLLATE "C" NOT NULL, properties jsonb NOT NULL CHECK(jsonb_typeof(properties)='object'),
 geom geometry CHECK(geom IS NULL OR (ST_SRID(geom)=4326 AND ST_NDims(geom) IN (2,3))), PRIMARY KEY(project,dataset,feature_id),
 FOREIGN KEY(project,dataset) REFERENCES _geoledger_center.datasets(project,id));
CREATE INDEX features_geom ON _geoledger_center.features USING gist(geom);
CREATE TABLE _geoledger_center.history (
 project uuid NOT NULL, dataset uuid NOT NULL, feature_id text COLLATE "C" NOT NULL, valid_from bigint NOT NULL, valid_to bigint,
 properties jsonb, geom geometry CHECK(geom IS NULL OR (ST_SRID(geom)=4326 AND ST_NDims(geom) IN (2,3))),
 PRIMARY KEY(project,dataset,feature_id,valid_from),
 FOREIGN KEY(project,dataset) REFERENCES _geoledger_center.datasets(project,id),
 FOREIGN KEY(project,valid_from) REFERENCES _geoledger_center.commits(project,revision),
 FOREIGN KEY(project,valid_to) REFERENCES _geoledger_center.commits(project,revision),
 CHECK(valid_to IS NULL OR valid_to>valid_from),
 CHECK(properties IS NULL OR jsonb_typeof(properties)='object'), CHECK(properties IS NOT NULL OR geom IS NULL));
CREATE UNIQUE INDEX history_live ON _geoledger_center.history(project,dataset,feature_id) WHERE valid_to IS NULL;
CREATE INDEX history_geom ON _geoledger_center.history USING gist(geom);
CREATE TABLE _geoledger_center.commit_changes (
 project uuid NOT NULL, revision bigint NOT NULL, dataset uuid NOT NULL, feature_id text COLLATE "C" NOT NULL,
 before_value jsonb, after_value jsonb, PRIMARY KEY(project,revision,dataset,feature_id),
 FOREIGN KEY(project,revision) REFERENCES _geoledger_center.commits(project,revision),
 FOREIGN KEY(project,dataset) REFERENCES _geoledger_center.datasets(project,id));
CREATE TABLE _geoledger_center.idempotency (
 project uuid NOT NULL, subject text NOT NULL, request_id uuid NOT NULL, payload jsonb NOT NULL, result jsonb NOT NULL,
 PRIMARY KEY(project,subject,request_id), FOREIGN KEY(project,subject) REFERENCES _geoledger_center.project_members(project,subject));
CREATE TABLE _geoledger_center.audit_events (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, project uuid NOT NULL REFERENCES _geoledger_center.projects(id),
 subject text NOT NULL, action text NOT NULL, detail jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(project,subject) REFERENCES _geoledger_center.project_members(project,subject));
CREATE FUNCTION _geoledger_center.immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable center record'; END $$;
CREATE TRIGGER commits_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.commits FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER changes_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.commit_changes FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER audit_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.audit_events FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER idempotency_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.idempotency FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE FUNCTION _geoledger_center.close_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='UPDATE' THEN
  IF OLD.valid_to IS NULL AND NEW.valid_to IS NOT NULL AND
     (to_jsonb(NEW)-'valid_to') = (to_jsonb(OLD)-'valid_to') THEN RETURN NEW; END IF;
 END IF;
 RAISE EXCEPTION 'immutable history value';
END $$;
CREATE TRIGGER history_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.history FOR EACH ROW EXECUTE FUNCTION _geoledger_center.close_history();

-- Managed tables use their own Dataset revision sequence and native value history.
CREATE TABLE _geoledger_center.collab_datasets (
 tenant text NOT NULL, project text NOT NULL, dataset text NOT NULL, epoch uuid PRIMARY KEY,
 source_schema text NOT NULL, source_table text NOT NULL, id_column text NOT NULL,
 geometry_column text NOT NULL, srid integer NOT NULL, head bigint NOT NULL DEFAULT 0,
 UNIQUE(tenant,project,dataset), UNIQUE(source_schema,source_table));
CREATE TABLE _geoledger_center.collab_schemas (
 epoch uuid NOT NULL REFERENCES _geoledger_center.collab_datasets(epoch), revision bigint NOT NULL,
 columns jsonb NOT NULL, PRIMARY KEY(epoch,revision));
CREATE TABLE _geoledger_center.collab_rows (
 epoch uuid NOT NULL REFERENCES _geoledger_center.collab_datasets(epoch), id text COLLATE "C" NOT NULL,
 revision bigint NOT NULL, value jsonb, feature jsonb, PRIMARY KEY(epoch,id,revision));
CREATE INDEX collab_rows_revision ON _geoledger_center.collab_rows(epoch,revision,id);
CREATE TABLE _geoledger_center.collab_workspaces (
 epoch uuid NOT NULL REFERENCES _geoledger_center.collab_datasets(epoch), id uuid PRIMARY KEY,
 owner text NOT NULL, base_revision bigint NOT NULL, version bigint NOT NULL DEFAULT 0,
 columns jsonb NOT NULL, resolved_schema_head bigint, closed boolean NOT NULL DEFAULT false);
CREATE TABLE _geoledger_center.collab_drafts (
 workspace uuid NOT NULL REFERENCES _geoledger_center.collab_workspaces(id), id text COLLATE "C" NOT NULL,
 value jsonb, resolved_head bigint, PRIMARY KEY(workspace,id));
CREATE TABLE _geoledger_center.collab_ids (
 workspace uuid NOT NULL REFERENCES _geoledger_center.collab_workspaces(id), client_id text NOT NULL,
 id text NOT NULL, PRIMARY KEY(workspace,client_id), UNIQUE(workspace,id));
CREATE TABLE _geoledger_center.collab_requests (
 epoch uuid NOT NULL REFERENCES _geoledger_center.collab_datasets(epoch), subject text NOT NULL,
 request_id uuid NOT NULL, payload jsonb NOT NULL, result jsonb NOT NULL,
 PRIMARY KEY(epoch,subject,request_id));
CREATE TABLE _geoledger_center.collab_commits (
 epoch uuid NOT NULL REFERENCES _geoledger_center.collab_datasets(epoch), revision bigint NOT NULL,
 subject text NOT NULL, message text NOT NULL, changes bigint NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(epoch,revision));
CREATE TRIGGER collab_rows_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.collab_rows FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER collab_schemas_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.collab_schemas FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER collab_commits_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.collab_commits FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
CREATE TRIGGER collab_requests_immutable BEFORE UPDATE OR DELETE ON _geoledger_center.collab_requests FOR EACH ROW EXECUTE FUNCTION _geoledger_center.immutable();
