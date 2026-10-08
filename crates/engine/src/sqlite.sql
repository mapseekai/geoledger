CREATE TABLE gl_format (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), version integer NOT NULL CHECK(version=5));
INSERT INTO gl_format VALUES(true,5);
CREATE TABLE gl_projects (
 id text PRIMARY KEY, name text NOT NULL, head INTEGER NOT NULL DEFAULT 0 CHECK(head>=0));
CREATE TABLE gl_project_members (
 project text REFERENCES gl_projects(id), subject text NOT NULL,
 role text NOT NULL CHECK(role IN ('owner','editor','viewer')), PRIMARY KEY(project,subject));
CREATE TABLE gl_datasets (
 project text REFERENCES gl_projects(id), id text NOT NULL, name text NOT NULL,
 PRIMARY KEY(project,id), UNIQUE(project,name));
CREATE TABLE gl_workspaces (
 project text NOT NULL, id text NOT NULL, owner text NOT NULL, base_revision INTEGER NOT NULL CHECK(base_revision>=0),
 version INTEGER NOT NULL DEFAULT 0 CHECK(version>=0), status text NOT NULL DEFAULT 'open' CHECK(status IN ('open','published','discarded')),
 PRIMARY KEY(project,id), FOREIGN KEY(project,owner) REFERENCES gl_project_members(project,subject));
CREATE TABLE gl_workspace_changes (
 project text NOT NULL, workspace text NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL,
 resolved_head INTEGER CHECK(resolved_head IS NULL OR resolved_head >= 0), resolution_stale boolean NOT NULL DEFAULT false, properties text, geom text,
 PRIMARY KEY(project,workspace,dataset,feature_id),
 FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 CHECK(properties IS NULL OR json_type(properties)='object'), CHECK(properties IS NOT NULL OR geom IS NULL));
CREATE TABLE gl_commits (
 project text NOT NULL REFERENCES gl_projects(id), revision INTEGER NOT NULL CHECK(revision>0),
 workspace text NOT NULL, subject text NOT NULL, message text NOT NULL, created_at text NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(project,revision), FOREIGN KEY(project,subject) REFERENCES gl_project_members(project,subject), FOREIGN KEY(project,workspace) REFERENCES gl_workspaces(project,id));
CREATE TABLE gl_history (
 project text NOT NULL, dataset text NOT NULL, feature_id text COLLATE BINARY NOT NULL, valid_from INTEGER NOT NULL, valid_to INTEGER,
 properties text, geom text,
 PRIMARY KEY(project,dataset,feature_id,valid_from),
 FOREIGN KEY(project,dataset) REFERENCES gl_datasets(project,id),
 FOREIGN KEY(project,valid_from) REFERENCES gl_commits(project,revision),
 FOREIGN KEY(project,valid_to) REFERENCES gl_commits(project,revision),
 CHECK(valid_to IS NULL OR valid_to>valid_from),
 CHECK(properties IS NULL OR json_type(properties)='object'), CHECK(properties IS NOT NULL OR geom IS NULL));
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
CREATE TRIGGER gl_commits_delete BEFORE DELETE ON gl_commits BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_commit_changes_update BEFORE UPDATE ON gl_commit_changes BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_commit_changes_delete BEFORE DELETE ON gl_commit_changes BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_audit_events_update BEFORE UPDATE ON gl_audit_events BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_audit_events_delete BEFORE DELETE ON gl_audit_events BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_idempotency_update BEFORE UPDATE ON gl_idempotency BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_idempotency_delete BEFORE DELETE ON gl_idempotency BEGIN SELECT RAISE(ABORT,'immutable version record'); END;
CREATE TRIGGER gl_history_update BEFORE UPDATE ON gl_history WHEN NOT (
OLD.valid_to IS NULL AND NEW.valid_to IS NOT NULL AND OLD.project=NEW.project AND OLD.dataset=NEW.dataset AND OLD.feature_id=NEW.feature_id AND OLD.valid_from=NEW.valid_from AND OLD.properties IS NEW.properties AND OLD.geom IS NEW.geom)
BEGIN SELECT RAISE(ABORT,'immutable history'); END;
CREATE TRIGGER gl_history_delete BEFORE DELETE ON gl_history BEGIN SELECT RAISE(ABORT,'immutable history'); END;
CREATE VIRTUAL TABLE gl_history_spatial USING rtree(id,min_x,max_x,min_y,max_y);
CREATE TRIGGER gl_history_spatial_insert AFTER INSERT ON gl_history WHEN NEW.geom IS NOT NULL AND gl_bound(NEW.geom,0) IS NOT NULL BEGIN
 INSERT INTO gl_history_spatial VALUES(NEW.rowid,gl_bound(NEW.geom,0),gl_bound(NEW.geom,2),gl_bound(NEW.geom,1),gl_bound(NEW.geom,3)); END;
CREATE TRIGGER gl_history_spatial_update AFTER UPDATE OF geom ON gl_history BEGIN
 DELETE FROM gl_history_spatial WHERE id=OLD.rowid;
 INSERT INTO gl_history_spatial SELECT NEW.rowid,gl_bound(NEW.geom,0),gl_bound(NEW.geom,2),gl_bound(NEW.geom,1),gl_bound(NEW.geom,3) WHERE NEW.geom IS NOT NULL AND gl_bound(NEW.geom,0) IS NOT NULL; END;
CREATE TRIGGER gl_history_spatial_delete AFTER DELETE ON gl_history BEGIN DELETE FROM gl_history_spatial WHERE id=OLD.rowid; END;
CREATE VIRTUAL TABLE gl_workspace_changes_spatial USING rtree(id,min_x,max_x,min_y,max_y);
CREATE TRIGGER gl_workspace_changes_spatial_insert AFTER INSERT ON gl_workspace_changes WHEN NEW.geom IS NOT NULL AND gl_bound(NEW.geom,0) IS NOT NULL BEGIN
 INSERT INTO gl_workspace_changes_spatial VALUES(NEW.rowid,gl_bound(NEW.geom,0),gl_bound(NEW.geom,2),gl_bound(NEW.geom,1),gl_bound(NEW.geom,3)); END;
CREATE TRIGGER gl_workspace_changes_spatial_update AFTER UPDATE OF geom ON gl_workspace_changes BEGIN
 DELETE FROM gl_workspace_changes_spatial WHERE id=OLD.rowid;
 INSERT INTO gl_workspace_changes_spatial SELECT NEW.rowid,gl_bound(NEW.geom,0),gl_bound(NEW.geom,2),gl_bound(NEW.geom,1),gl_bound(NEW.geom,3) WHERE NEW.geom IS NOT NULL AND gl_bound(NEW.geom,0) IS NOT NULL; END;
CREATE TRIGGER gl_workspace_changes_spatial_delete AFTER DELETE ON gl_workspace_changes BEGIN DELETE FROM gl_workspace_changes_spatial WHERE id=OLD.rowid; END;
