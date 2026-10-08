-- Format 6: membership removal, project lifecycle, open-ended format check.
CREATE TABLE gl_format_next (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), version integer NOT NULL CHECK(version>=5));
INSERT INTO gl_format_next SELECT singleton, 6 FROM gl_format;
DROP TABLE gl_format;
ALTER TABLE gl_format_next RENAME TO gl_format;
ALTER TABLE gl_project_members ADD COLUMN removed boolean NOT NULL DEFAULT false;
ALTER TABLE gl_projects ADD COLUMN state text NOT NULL DEFAULT 'active' CHECK(state IN ('active','archived','deleted'));
CREATE INDEX gl_project_members_subject ON gl_project_members(subject,project);
