use super::*;

impl DatasetRepository for SqlTransaction {
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>> {
        let key = (project.to_owned(), dataset.to_owned());
        if let Some(row) = self.1.datasets.get(&key) {
            return Ok(Some(row.clone()));
        }
        let row = self.query_opt(
            "SELECT geometry_type,name,coordinate_dimension,postgis_source,crs FROM gl_datasets WHERE project=$1 AND id=$2",
            &[&project, &dataset],
        )?;
        if let Some(row) = &row {
            if self.1.datasets.len() >= 128 {
                self.1.datasets.clear();
            }
            self.1.datasets.insert(key, row.clone());
        }
        Ok(row)
    }

    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
        crs: &str,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_datasets(project,id,name,geometry_type,coordinate_dimension,crs) VALUES($1,$2,$3,$4,$5,$6)",
            &[
                &project,
                &dataset,
                &name,
                &geometry_type,
                &coordinate_dimension,
                &crs,
            ],
        )
    }

    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()> {
        self.1.datasets.clear();
        self.execute(
            "UPDATE gl_datasets SET name=$3 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &name],
        )
    }

    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query(
            "SELECT id,name,geometry_type,coordinate_dimension,postgis_source,crs FROM gl_datasets WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3",
            &[&project, &after, &limit],
        )
    }
}
