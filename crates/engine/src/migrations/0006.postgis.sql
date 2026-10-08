-- Format 6: membership removal, project lifecycle, open-ended format check.
ALTER TABLE gl_format DROP CONSTRAINT gl_format_version_check;
ALTER TABLE gl_format ADD CONSTRAINT gl_format_version_check CHECK(version>=5);
ALTER TABLE gl_project_members ADD COLUMN removed boolean NOT NULL DEFAULT false;
ALTER TABLE gl_projects ADD COLUMN state text NOT NULL DEFAULT 'active' CHECK(state IN ('active','archived','deleted'));
CREATE INDEX gl_project_members_subject ON gl_project_members(subject,project);
UPDATE gl_format SET version=6;
