//! Deadline-aware synchronous database façade. Dropping a timed-out connection
//! aborts its driver and shuts down its runtime before releasing capacity.
use crate::{Error, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;
use tokio_postgres::{Row, Statement, types::ToSql};

pub(crate) const CAPACITY: usize = 20;
#[derive(Default)]
pub(crate) struct Pool {
    state: Mutex<State>,
    available: Condvar,
}
#[derive(Default)]
struct State {
    active: usize,
    idle: Vec<Connection>,
}
struct Connection {
    runtime: Option<Runtime>,
    client: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<()>,
    statements: HashMap<String, Statement>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
        // Current-thread runtime owns no blocking jobs. Shutdown drops every
        // socket-owning driver, also when the pool is dropped inside async HTTP.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
impl Pool {
    pub fn connect(self: &Arc<Self>, dsn: &str, timeout: Duration) -> Result<Client> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::new(400, "operation timeout out of range"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new(500, "pool unavailable"))?;
        while state.active == CAPACITY {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(expired());
            }
            state = self
                .available
                .wait_timeout(state, left)
                .map_err(|_| Error::new(500, "pool unavailable"))?
                .0;
        }
        state.active += 1;
        let connection = state.idle.pop();
        drop(state);
        let mut session = Client {
            pool: self.clone(),
            connection,
            deadline,
            reusable: true,
        };
        if session
            .connection
            .as_ref()
            .is_some_and(|c| c.client.is_closed())
        {
            session.connection = None;
        }
        if session.connection.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| {
                    Error::new(503, "database runtime unavailable").caused_by(source)
                })?;
            let settings = super::pgtls::PgSettings::parse(dsn)?;
            let tls = settings.connector()?;
            let connected = runtime.block_on(async {
                tokio::time::timeout_at(
                    deadline.into(),
                    settings
                        .config
                        .connect(postgres_native_tls::MakeTlsConnector::new(tls)),
                )
                .await
                .map_err(|_| expired())?
                .map_err(Error::database)
            });
            let (client, connection) = match connected {
                Ok(connected) => connected,
                Err(error) => {
                    runtime.shutdown_background();
                    return Err(error);
                }
            };
            let driver = runtime.spawn(async move {
                let _ = connection.await;
            });
            session.connection = Some(Connection {
                runtime: Some(runtime),
                client,
                driver,
                statements: HashMap::new(),
            });
            session.batch_execute("SET statement_timeout='30s'; SET lock_timeout='10s'; SET search_path=public,pg_catalog")?;
        }
        Ok(session)
    }
}
fn expired() -> Error {
    Error::new(
        504,
        "database operation deadline exceeded; retry publication with the same request_id",
    )
}
pub(crate) struct Client {
    pool: Arc<Pool>,
    connection: Option<Connection>,
    deadline: Instant,
    reusable: bool,
}
impl Drop for Client {
    fn drop(&mut self) {
        let connection = self.connection.take();
        if let Ok(mut state) = self.pool.state.lock() {
            if self.reusable {
                if let Some(c) = connection {
                    state.idle.push(c);
                }
            } else {
                drop(connection);
            }
            state.active -= 1;
            self.pool.available.notify_one();
        }
    }
}
impl Client {
    fn check_deadline(&mut self) -> Result<()> {
        if Instant::now() >= self.deadline {
            self.reusable = false;
            self.connection = None;
            return Err(expired());
        }
        Ok(())
    }
    fn statement(&mut self, sql: &str) -> Result<Statement> {
        self.check_deadline()?;
        if let Some(s) = self.connection.as_ref().and_then(|c| c.statements.get(sql)) {
            return Ok(s.clone());
        }
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.prepare(sql)).await
        });
        let statement = self.finish(result)?;
        if let Some(c) = self.connection.as_mut() {
            if c.statements.len() >= 128 {
                c.statements.clear();
            }
            c.statements.insert(sql.to_owned(), statement.clone());
        }
        Ok(statement)
    }
    pub fn query(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>> {
        let statement = self.statement(sql)?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.query(&statement, params)).await
        });
        self.finish(result)
    }
    fn finish<T>(
        &mut self,
        result: std::result::Result<
            std::result::Result<T, tokio_postgres::Error>,
            tokio::time::error::Elapsed,
        >,
    ) -> Result<T> {
        match result {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                self.reusable = false;
                Err(e.into())
            }
            Err(source) => {
                self.reusable = false;
                self.connection = None;
                Err(expired().caused_by(source))
            }
        }
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.check_deadline()?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.batch_execute(sql)).await
        });
        self.finish(result)
    }
    pub(super) fn invalidate(&mut self) {
        self.reusable = false;
        self.connection = None;
    }
}
