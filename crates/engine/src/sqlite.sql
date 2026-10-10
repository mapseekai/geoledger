CREATE TABLE gl_purge (project text PRIMARY KEY);
CREATE TABLE gl_format (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), version integer NOT NULL CHECK(version=12));
INSERT INTO gl_format VALUES(true,12);
CREATE TABLE gl_projects (
 id text PRIMARY KEY, name text NOT NULL, head INTEGER NOT NULL DEFAULT 0 CHECK(head>=0),
 state text NOT NULL DEFAULT 'active' CHECK(state IN ('active','archived','deleted')));
CREATE TABLE gl_project_members (
 project text REFERENCES gl_projects(id), subject text NOT NULL,
 role text NOT NULL CHECK(role IN ('owner','editor','viewer')), removed boolean NOT NULL DEFAULT false,
 PRIMARY KEY(project,subject));
CREATE INDEX gl_project_members_subject ON gl_project_members(subject,project);
CREATE TABLE gl_datasets (
 project text REFERENCES gl_projects(id), id text NOT NULL, name text NOT NULL,
 geometry_type text NOT NULL CHECK(geometry_type IN ('point','line','polygon')),
 coordinate_dimension integer NOT NULL DEFAULT 2 CHECK(coordinate_dimension IN (2,3)),
 postgis_source text,
 PRIMARY KEY(project,id), UNIQUE(project,name));
CREATE TABLE gl_workspaces (
 project text NOT NULL, id text NOT NULL, owner text NOT NULL, base_revision INTEGER NOT NULL CHECK(base_revision>=0),
 version INTEGER NOT NULL DEFAULT 0 CHECK(version>=0), status text NOT NULL DEFAULT 'open' CHECK(status IN ('open','published','discarded')),
 PRIMARY KEY(project,id), FOREIGN KEY(project,owner) REFERENCES gl_project_members(project,subject));
CREATE TABLE gl_workspace_changes (
 project text NOT NULL, workspace text NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL,
 resolved_head INTEGER CHECK(resolved_head IS NULL OR resolved_head >= 0), resolution_stale boolean NOT NULL DEFAULT false, properties text, geometry_json text, geom BLOB, geom_z BLOB,
 PRIMARY KEY(project,workspace,dataset,feature_id),
 FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 CHECK(properties IS NULL OR json_type(properties)='object'), CHECK(properties IS NOT NULL OR geometry_json IS NULL));
CREATE INDEX workspace_active_resolutions ON gl_workspace_changes(project,workspace) WHERE resolved_head IS NOT NULL AND NOT resolution_stale;
CREATE TABLE gl_commits (
 project text NOT NULL REFERENCES gl_projects(id), revision INTEGER NOT NULL CHECK(revision>0),
 workspace text NOT NULL, subject text NOT NULL, message text NOT NULL, created_at text NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(project,revision), FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject), FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id));
CREATE TABLE gl_history (
 project text NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL, valid_from INTEGER NOT NULL, valid_to INTEGER,
 properties text, geometry_json text, geom BLOB, geom_z BLOB,
 PRIMARY KEY(project,dataset,feature_id,valid_from),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 FOREIGN KEY(project,valid_from) REFERENCES gl_commits(project,revision),
 FOREIGN KEY(project,valid_to) REFERENCES gl_commits(project,revision),
 CHECK(valid_to IS NULL OR valid_to>valid_from),
 CHECK(properties IS NULL OR json_type(properties)='object'), CHECK(properties IS NOT NULL OR geometry_json IS NULL));
CREATE UNIQUE INDEX history_live ON gl_history(project,dataset,feature_id) WHERE valid_to IS NULL;
CREATE TABLE gl_commit_changes (
 project text NOT NULL, revision INTEGER NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL,
 before_value text, after_value text, PRIMARY KEY(project,revision,dataset,feature_id),
 FOREIGN KEY(project,revision) REFERENCES gl_commits(project,revision),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id));
CREATE TABLE gl_idempotency (
 project text NOT NULL, subject text NOT NULL, request_id text NOT NULL, payload text NOT NULL, result text NOT NULL,
 PRIMARY KEY(project,subject,request_id), FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject));
CREATE TABLE gl_audit_events (
 id INTEGER  PRIMARY KEY, project text NOT NULL REFERENCES gl_projects(id),
 subject text NOT NULL, action text NOT NULL, detail text NOT NULL, created_at text NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject));
CREATE INDEX audit_project_id ON gl_audit_events(project,id);
CREATE TRIGGER gl_commits_update BEFORE UPDATE ON gl_commits BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_commits_delete BEFORE DELETE ON gl_commits WHEN NOT EXISTS (SELECT 1 FROM gl_purge WHERE project=OLD.project) BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_commit_changes_update BEFORE UPDATE ON gl_commit_changes BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_commit_changes_delete BEFORE DELETE ON gl_commit_changes WHEN NOT EXISTS (SELECT 1 FROM gl_purge WHERE project=OLD.project) BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_audit_events_update BEFORE UPDATE ON gl_audit_events BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_audit_events_delete BEFORE DELETE ON gl_audit_events BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_idempotency_update BEFORE UPDATE ON gl_idempotency BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_idempotency_delete BEFORE DELETE ON gl_idempotency WHEN NOT EXISTS (SELECT 1 FROM gl_purge WHERE project=OLD.project) BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_history_update BEFORE UPDATE ON gl_history WHEN NOT (
OLD.valid_to IS NULL AND NEW.valid_to IS NOT NULL AND OLD.project=NEW.project AND OLD.dataset=NEW.dataset AND OLD.feature_id=NEW.feature_id AND OLD.valid_from=NEW.valid_from AND OLD.properties IS NEW.properties AND OLD.geometry_json IS NEW.geometry_json AND OLD.geom IS NEW.geom AND OLD.geom_z IS NEW.geom_z)
BEGIN SELECT RAISE(ABORT,'immutable history'); END;
CREATE TRIGGER gl_history_delete BEFORE DELETE ON gl_history WHEN NOT EXISTS (SELECT 1 FROM gl_purge WHERE project=OLD.project) BEGIN SELECT RAISE(ABORT,'immutable history'); END;

-- Derived transactional state. These tables are rebuilt during logical import.
CREATE TABLE gl_workspace_sizes (
 project text NOT NULL, workspace text NOT NULL, changes bigint NOT NULL DEFAULT 0 CHECK(changes>=0),
 PRIMARY KEY(project,workspace), FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id) ON DELETE CASCADE);
CREATE TABLE gl_conflict_cache (
 project text NOT NULL, workspace text NOT NULL, base_revision bigint NOT NULL, workspace_version bigint NOT NULL,
 head bigint NOT NULL, total bigint NOT NULL DEFAULT 0, complete boolean NOT NULL DEFAULT false,
 PRIMARY KEY(project,workspace), FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id) ON DELETE CASCADE);
CREATE TABLE gl_conflict_keys (
 project text NOT NULL, workspace text NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL,
 PRIMARY KEY(project,workspace,dataset,feature_id),
 FOREIGN KEY(project,workspace) REFERENCES gl_conflict_cache(project,workspace) ON DELETE CASCADE);

CREATE TRIGGER gl_workspace_size_init AFTER INSERT ON gl_workspaces BEGIN
 INSERT INTO gl_workspace_sizes(project,workspace) VALUES(NEW.project,NEW.id); END;
CREATE TRIGGER gl_workspace_size_insert AFTER INSERT ON gl_workspace_changes BEGIN
 UPDATE gl_workspace_sizes SET changes=changes+1 WHERE project=NEW.project AND workspace=NEW.workspace; END;
CREATE TRIGGER gl_workspace_size_delete AFTER DELETE ON gl_workspace_changes BEGIN
 UPDATE gl_workspace_sizes SET changes=changes-1 WHERE project=OLD.project AND workspace=OLD.workspace; END;
