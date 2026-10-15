#![allow(clippy::unwrap_used)]
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn sqlite_empty_bbox_and_invalidation_do_not_scan_unrelated_drafts() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::Sqlite(dir.path().join("query.db"));
    let pool = Arc::new(postgres::Pool::default());
    Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
    t.batch_execute(
        r#"INSERT INTO gl_projects(id,name) VALUES('p','p');
        INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner');
        INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','d','point');
        INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);
        INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed');
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom)
        VALUES('p','d','seed',1,'{}','{"type":"Point","coordinates":[20,20]}',SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20]}'),0));
        ANALYZE gl_history;
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) VALUES('p','w','d','seed','{}');
        ANALYZE gl_workspace_changes;
        WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<2000)
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties)
        SELECT 'p','w','d',printf('%08d',n),'{}' FROM ids;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties)
        SELECT project,dataset,feature_id,1,properties FROM gl_workspace_changes WHERE feature_id<>'seed';
        UPDATE gl_workspace_changes SET resolved_head=0 WHERE feature_id='00000001';"#,
    )?;
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = steps.clone();
    if let Backend::Sqlite(c) = &t.0.backend {
        let c = c.read().map_err(sqlite_error)?;
        c.progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .map_err(sqlite_error)?;
    }
    assert!(
        !t.edit_base("p", "w", "d", "absent", 1)?
            .get::<_, bool>(0usize)?
    );
    let lookup_steps = steps.swap(0, Ordering::Relaxed);
    assert!(
        lookup_steps < 1000,
        "stale ANALYZE caused resolution lookup scan: {lookup_steps}"
    );
    t.batch_execute(r#"WITH RECURSIVE ids(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM ids WHERE n<1000)
        INSERT INTO gl_commits(project,revision,workspace,subject,message) SELECT 'p',n,'w','alice','history' FROM ids;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties)
        SELECT 'p','d','versions',revision,CASE WHEN revision<1000 THEN revision+1 END,'{"v":'||revision||'}'
        FROM gl_commits WHERE project='p';"#)?;
    steps.store(0, Ordering::Relaxed);
    let historical = t.edit_base("p", "w", "d", "versions", 500)?;
    assert_eq!(historical.get::<_, String>(1usize)?, r#"{"v":500}"#);
    assert!(
        steps.swap(0, Ordering::Relaxed) < 1000,
        "base lookup scanned earlier versions"
    );
    t.remove_delta("p", "w", "d", "absent")?;
    let delete_steps = steps.swap(0, Ordering::Relaxed);
    assert!(
        delete_steps < 1000,
        "stale ANALYZE caused delta deletion scan: {delete_steps}"
    );
    t.clear_deltas("p", "absent")?;
    let clear_steps = steps.swap(0, Ordering::Relaxed);
    assert!(
        clear_steps < 1000,
        "stale ANALYZE caused workspace deletion scan: {clear_steps}"
    );
    t.invalidate_resolutions("p", "w")?;
    let invalidation_steps = steps.swap(0, Ordering::Relaxed);
    assert!(
        invalidation_steps < 1000,
        "invalidation scanned draft rows: {invalidation_steps}"
    );
    assert_eq!(
        t.query_one("SELECT changes()", &[])?
            .get::<_, i64>(0usize)?,
        1
    );
    t.invalidate_resolutions("p", "w")?;
    assert_eq!(
        t.query_one("SELECT changes()", &[])?
            .get::<_, i64>(0usize)?,
        0
    );
    steps.store(0, Ordering::Relaxed);
    let rows = t.feature_page(&crate::repository::FeatureQuery {
        project: "p".into(),
        dataset: "d".into(),
        revision: 1,
        workspace: Some("w".into()),
        after: String::new(),
        feature_id: None,
        bbox: Some([-1., -1., 1., 1.]),
        limit: 32,
    })?;
    let query_steps = steps.load(Ordering::Relaxed);
    assert!(rows.is_empty());
    assert!(
        query_steps < 1000,
        "empty Rtree query scanned draft rows: {query_steps}"
    );
    t.batch_execute(r#"UPDATE gl_workspace_changes SET geom=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[0,0]}'),0),geometry_json='{"type":"Point","coordinates":[0,0]}'"#)?;
    let dense = crate::repository::FeatureQuery {
        project: "p".into(),
        dataset: "d".into(),
        revision: 1,
        workspace: Some("w".into()),
        after: String::new(),
        feature_id: None,
        bbox: Some([-180., -90., 180., 90.]),
        limit: 20,
    };
    assert!(!t.sparse_spatial_candidates("workspace_changes", &dense)?);
    steps.store(0, Ordering::Relaxed);
    let rows = t.feature_page(&dense)?;
    let dense_steps = steps.load(Ordering::Relaxed);
    assert_eq!(rows.len(), 20);
    assert_eq!(rows[0].get::<_, String>(0usize)?, "00000001");
    assert!(
        dense_steps < 5000,
        "dense bbox enumerated all candidates: {dense_steps}"
    );
    t.batch_execute(r#"UPDATE gl_workspace_changes SET geom=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20]}'),0),geom_z=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20,3]}'),0) WHERE feature_id='00002000'"#)?;
    let sparse = crate::repository::FeatureQuery {
        bbox: Some([19., 19., 21., 21.]),
        ..dense
    };
    steps.store(0, Ordering::Relaxed);
    let rows = t.feature_page(&sparse)?;
    let sparse_steps = steps.load(Ordering::Relaxed);
    assert_eq!(
        rows.len(),
        1,
        "XY/XYZ candidates for one row must be deduplicated"
    );
    assert_eq!(rows[0].get::<_, String>(0usize)?, "00002000");
    assert!(
        sparse_steps < 1000,
        "stale ANALYZE caused sparse candidate scan: {sparse_steps}"
    );
    let historical = crate::repository::FeatureQuery {
        workspace: None,
        ..sparse
    };
    steps.store(0, Ordering::Relaxed);
    let rows = t.feature_page(&historical)?;
    let history_steps = steps.load(Ordering::Relaxed);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, String>(0usize)?, "seed");
    assert!(
        history_steps < 1000,
        "stale ANALYZE caused history candidate scan: {history_steps}"
    );
    Ok(())
}

#[test]
fn sqlite_sparse_bbox_above_page_cap_uses_candidates_in_scoped_history_and_drafts() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::Sqlite(dir.path().join("sparse.db"));
    let pool = Arc::new(postgres::Pool::default());
    Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
    t.batch_execute(r#"
        INSERT INTO gl_projects(id,name) VALUES('p','p');
        INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner');
        INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','d','point'),('p','other','other','point');
        INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);
        INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed'),('p',2,'w','alice','later');
        WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<50000)
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom)
        SELECT 'p','d',printf('%08d',n),1,'{}','{"type":"Point","coordinates":[20,20]}',MakePoint(CASE WHEN n>49900 THEN 0 ELSE 20 END,CASE WHEN n>49900 THEN 0 ELSE 20 END,0) FROM ids;
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom)
        SELECT project,'w',dataset,feature_id,properties,geom FROM gl_history;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom)
        SELECT project,'other',feature_id,1,properties,MakePoint(0,0,0) FROM gl_history WHERE feature_id<='00000100';
        UPDATE gl_history SET valid_to=2 WHERE dataset='d' AND feature_id<='00000100';
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom)
        SELECT project,dataset,feature_id,2,properties,MakePoint(0,0,0) FROM gl_history WHERE dataset='d' AND feature_id<='00000100';
    "#)?;
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = steps.clone();
    if let Backend::Sqlite(c) = &t.0.backend {
        let c = c.read().map_err(sqlite_error)?;
        c.progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .map_err(sqlite_error)?;
    }
    for workspace in [None, Some("w".to_owned())] {
        let query = crate::repository::FeatureQuery {
            project: "p".into(),
            dataset: "d".into(),
            revision: 1,
            workspace,
            after: String::new(),
            feature_id: None,
            bbox: Some([-1., -1., 1., 1.]),
            limit: 20,
        };
        steps.store(0, Ordering::Relaxed);
        let rows = t.feature_page(&query)?;
        let query_steps = steps.load(Ordering::Relaxed);
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0].get::<_, String>(0usize)?, "00049901");
        assert!(
            query_steps < 30000,
            "sparse bbox scanned background: {query_steps}"
        );
        let next = crate::repository::FeatureQuery {
            after: "00049980".into(),
            ..query
        };
        let rows = t.feature_page(&next)?;
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0].get::<_, String>(0usize)?, "00049981");
    }
    Ok(())
}

#[test]
fn sqlite_publication_closes_only_changed_history_keys() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::Sqlite(dir.path().join("close-history.db"));
    let pool = Arc::new(postgres::Pool::default());
    Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
    t.batch_execute("INSERT INTO gl_projects(id,name) VALUES('p','p'); INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0); INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed'),('p',2,'w','alice','edit'); WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<5000) INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT 'p','d',printf('%08d',n),1,'{}' FROM ids;")?;
    t.begin_merge()?;
    t.stage_merge("d", "00001000", &Some("{}".into()), &None)?;
    t.flush_stage()?;
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = steps.clone();
    if let Backend::Sqlite(c) = &t.0.backend {
        c.read()
            .map_err(sqlite_error)?
            .progress_handler(
                1,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .map_err(sqlite_error)?;
    }
    t.close_history("p", 2)?;
    let work = steps.load(Ordering::Relaxed);
    assert!(
        work < 1000,
        "closing one feature scanned unrelated history: {work}"
    );
    assert_eq!(
        t.query_one("SELECT count(*) FROM gl_history WHERE valid_to=2", &[])?
            .get::<_, i64>(0usize)?,
        1
    );
    assert_eq!(
        t.query_one(
            "SELECT count(*) FROM gl_history WHERE feature_id='00001001' AND valid_to IS NULL",
            &[]
        )?
        .get::<_, i64>(0usize)?,
        1
    );
    Ok(())
}
