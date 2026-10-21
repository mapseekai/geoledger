use super::*;

impl AuditRepository for SqlTransaction {
    fn append_audit(
        &mut self,
        project: &str,
        subject: &str,
        action: &str,
        detail: &str,
    ) -> Result<()> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Hold through commit so project cursors cannot skip a lower, uncommitted ID.
            self.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,7882))::text",
                &[&project],
            )?;
        }
        self.execute(
            "INSERT INTO gl_audit_events(project,subject,action,detail) VALUES($1,$2,$3,$4)",
            &[&project, &subject, &action, &detail],
        )
    }

    fn audit_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT id,subject,action,detail,created_at FROM gl_audit_events WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3", &[&project, &after, &limit])
    }

    fn audit_has_after(&mut self, project: &str, after: i64) -> Result<bool> {
        self.query_one(
            "SELECT EXISTS(SELECT 1 FROM gl_audit_events WHERE project=$1 AND id>$2)",
            &[&project, &after],
        )?
        .get(0usize)
    }
}
