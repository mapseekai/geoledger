use super::Row;
use crate::Result;

/// Optional business-table enrollment on the same transaction as its initial publication.
pub trait PostgisTables: Send {
    /// Validate and lock a business table; returns (geometry family, dimension, CRS).
    fn inspect_postgis_table(&mut self, source: &crate::PostgisTable) -> Result<Row> {
        let _ = source;
        Err(crate::Error::new(
            400,
            "business tables require the PostGIS backend",
        ))
    }
    /// Source rows (id, properties JSON, geometry JSON), in native primary-key order.
    /// An empty cursor starts the scan; subsequent cursors are the last returned id.
    fn postgis_table_page(
        &mut self,
        source: &crate::PostgisTable,
        after: &str,
    ) -> Result<Vec<Row>> {
        let _ = (source, after);
        Err(crate::Error::new(
            400,
            "business tables require the PostGIS backend",
        ))
    }
    fn bind_postgis_table(
        &mut self,
        project: &str,
        dataset: &str,
        source: &crate::PostgisTable,
        revision: i64,
    ) -> Result<()> {
        let _ = (project, dataset, source, revision);
        Err(crate::Error::new(
            400,
            "business tables require the PostGIS backend",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unsupported;
    impl PostgisTables for Unsupported {}

    #[test]
    fn unsupported_business_tables_keep_explicit_errors() {
        let mut backend = Unsupported;
        let table = crate::PostgisTable {
            schema: "business".into(),
            table: "roads".into(),
            id_column: "id".into(),
            geometry_column: "geom".into(),
        };
        for error in [
            backend.inspect_postgis_table(&table).err(),
            backend.postgis_table_page(&table, "").err(),
            backend.bind_postgis_table("p", "d", &table, 1).err(),
        ] {
            assert!(matches!(error, Some(e) if e.status == 400));
        }
    }
}
