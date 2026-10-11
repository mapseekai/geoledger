//! Deadline-aware synchronous database façade. Dropping a timed-out connection
//! retains capacity until rollback is acknowledged. Unknown cleanup outcomes fail closed.
use crate::{Error, Result};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;
use tokio_postgres::{Row, Statement, types::ToSql};

pub(crate) struct Pool {
    state: Mutex<State>,
    available: Condvar,
    recovery: Mutex<()>,
    options: crate::StorageOptions,
    wait_timeouts: AtomicU64,
}
impl Default for Pool {
    fn default() -> Self {
        Self::new(crate::StorageOptions::default())
    }
}
#[derive(Default)]
struct State {
    active: usize,
    // Avoid allocating a large Connection on every checkout or moving it in the session enum.
    #[allow(clippy::vec_box)]
    idle: Vec<Box<Connection>>,
    retiring: usize,
}
struct Connection {
    runtime: Option<Runtime>,
    client: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<()>,
    statements: HashMap<String, Statement>,
    tls: Arc<native_tls::TlsConnector>,
    config: Arc<tokio_postgres::Config>,
    identity: Option<(i32, String)>,
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
    pub fn new(options: crate::StorageOptions) -> Self {
        Self {
            state: Mutex::default(),
            available: Condvar::new(),
            recovery: Mutex::new(()),
            options,
            wait_timeouts: AtomicU64::new(0),
        }
    }
    pub fn stats(&self) -> crate::PoolStats {
        let (active, idle, retiring) = self
            .state
            .lock()
            .map(|s| (s.active, s.idle.len(), s.retiring))
            .unwrap_or_default();
        crate::PoolStats {
            capacity: self.options.pool_size,
            active,
            idle,
            retiring,
            wait_timeouts: self.wait_timeouts.load(Ordering::Relaxed),
        }
    }
    pub fn connect(self: &Arc<Self>, dsn: &str, timeout: Duration) -> Result<Client> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::new(400, "operation timeout out of range"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new(500, "pool unavailable"))?;
        while state.active >= self.options.pool_size {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.wait_timeouts.fetch_add(1, Ordering::Relaxed);
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
            result_limit: usize::MAX,
            response_reservation: None,
            statement_deadline: None,
        };
        // Idle drivers use a current-thread runtime. Poll a protocol-level health
        // check before handing an idle session to a business transaction.
        if let Some(c) = session.connection.as_ref() {
            let healthy = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
                tokio::time::timeout_at(deadline.into(), c.client.check_connection()).await
            });
            match healthy {
                Ok(Ok(())) => {}
                Ok(Err(_)) => session.connection = None, // no in-flight business transaction
                Err(_) => {
                    session.connection = None;
                    return Err(expired());
                }
            }
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
                        .connect(postgres_native_tls::MakeTlsConnector::new(tls.clone())),
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
            session.connection = Some(Box::new(Connection {
                runtime: Some(runtime),
                client,
                driver,
                statements: HashMap::new(),
                tls: Arc::new(tls),
                config: Arc::new(settings.config.clone()),
                identity: None,
            }));
            let settings = format!(
                "SET statement_timeout='{}ms'; SET lock_timeout='{}ms'; SET search_path=public,pg_catalog",
                self.options.statement_timeout.as_millis(),
                self.options.lock_timeout.as_millis()
            );
            session.batch_execute(&settings)?;
            let rows = session.query("SELECT pg_backend_pid(),extract(epoch FROM backend_start)::text FROM pg_stat_activity WHERE pid=pg_backend_pid()", &[])?;
            if let Some(row) = rows.first()
                && let Some(connection) = session.connection.as_mut()
            {
                connection.identity = Some((row.try_get(0)?, row.try_get(1)?));
            }
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
    connection: Option<Box<Connection>>,
    deadline: Instant,
    reusable: bool,
    result_limit: usize,
    response_reservation: Option<crate::ResponseReservation>,
    statement_deadline: Option<Instant>,
}
impl Drop for Client {
    fn drop(&mut self) {
        let connection = self.connection.take();
        if !self.reusable
            && let Some(connection) = connection
        {
            // Transfer ownership of the existing pool slot to bounded retirement.
            // No replacement is admitted until a ROLLBACK response proves the
            // backend has finished prior commands (including an uncertain COMMIT).
            if let Ok(mut state) = self.pool.state.lock() {
                state.retiring += 1;
            }
            let pool = Arc::downgrade(&self.pool);
            let started = std::thread::Builder::new()
                .name("geoledger-db-retire".into())
                .spawn(move || {
                    let mut reported = false;
                    loop {
                        let Some(runtime) = connection.runtime.as_ref() else {
                            return;
                        };
                        let token = connection.client.cancel_token();
                        let tls = postgres_native_tls::MakeTlsConnector::new(
                            connection.tls.as_ref().clone(),
                        );
                        let result = runtime.block_on(async {
                            tokio::time::timeout(Duration::from_secs(1), async {
                                // Cancellation alone is not confirmation; always drain a rollback.
                                let _ = token.cancel_query(tls).await;
                                connection.client.batch_execute("ROLLBACK").await
                            })
                            .await
                        });
                        if matches!(result, Ok(Ok(()))) {
                            break;
                        }
                        if connection.driver.is_finished()
                            && let Some(owner) = pool.upgrade()
                            && let Ok(_probe) = owner.recovery.try_lock()
                            && connection.backend_gone()
                        {
                            break;
                        }
                        if pool.strong_count() == 0 {
                            return;
                        }
                        if !reported {
                            tracing::warn!(
                                "database cleanup unconfirmed; pool slot remains quarantined"
                            );
                            reported = true;
                        }
                        // An unavailable backend must not cause unbounded replacement sessions.
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    connection.driver.abort();
                    drop(connection);
                    if let Some(pool) = pool.upgrade()
                        && let Ok(mut state) = pool.state.lock()
                    {
                        state.active -= 1;
                        state.retiring -= 1;
                        pool.available.notify_all();
                    }
                });
            if let Err(error) = started {
                // Fail closed rather than pretending a possibly running backend was released.
                tracing::error!(%error, "cannot start database cleanup; pool slot quarantined");
            }
            return;
        }
        if let Ok(mut state) = self.pool.state.lock() {
            if let Some(connection) = connection {
                state.idle.push(connection);
            }
            state.active -= 1;
            self.pool.available.notify_one();
        }
    }
}
impl Client {
    pub fn set_response_reservation(&mut self, reservation: Option<crate::ResponseReservation>) {
        self.response_reservation = reservation;
    }
    pub fn set_result_limit(&mut self, bytes: usize) {
        self.result_limit = bytes;
    }
    fn check_deadline(&mut self) -> Result<()> {
        if Instant::now() >= self.deadline {
            self.reusable = false;
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
        self.apply_budget()?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let limit = self.result_limit;
        let reserve = self.response_reservation.clone();
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), async {
                use tokio_stream::StreamExt;
                let stream = c
                    .client
                    .query_raw(&statement, params.iter().copied())
                    .await?;
                let mut stream = std::pin::pin!(stream);
                let mut rows = Vec::new();
                let mut bytes = 0usize;
                while let Some(row) = stream.try_next().await? {
                    bytes = bytes.saturating_add(row.raw_size_bytes());
                    if bytes > limit {
                        return Ok::<_, tokio_postgres::Error>((
                            Vec::new(),
                            Some(Error::new(413, "database result batch exceeds byte budget")),
                        ));
                    }
                    if let Some(reserve) = &reserve
                        && let Err(error) = reserve(bytes)
                    {
                        return Ok((Vec::new(), Some(error)));
                    }
                    rows.push(row);
                }
                Ok((rows, None))
            })
            .await
        });
        let (rows, exceeded) = self.finish(result)?;
        if let Some(error) = exceeded {
            self.reusable = false;
            return Err(error);
        }
        Ok(rows)
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
                if e.code() == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
                    && self
                        .statement_deadline
                        .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    Err(Error::new(504, "database statement deadline exceeded").caused_by(e))
                } else {
                    Err(e.into())
                }
            }
            Err(source) => {
                self.reusable = false;
                Err(expired().caused_by(source))
            }
        }
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.apply_budget()?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.batch_execute(sql)).await
        });
        self.finish(result)
    }
    // Server-side limits track the remaining operation deadline, never a fresh
    // full timeout for every statement. Values are integers, not untrusted SQL.
    fn apply_budget(&mut self) -> Result<()> {
        self.check_deadline()?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let bounded = |configured: Duration| {
            let value = if configured.is_zero() {
                remaining
            } else {
                remaining.min(configured)
            };
            value.as_millis().clamp(1, i32::MAX as u128)
        };
        let statement_ms = bounded(self.pool.options.statement_timeout);
        let lock_ms = bounded(self.pool.options.lock_timeout);
        self.statement_deadline =
            Instant::now().checked_add(Duration::from_millis(statement_ms as u64));
        let sql =
            format!("SET statement_timeout='{statement_ms}ms'; SET lock_timeout='{lock_ms}ms'");
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.batch_execute(&sql)).await
        });
        self.finish(result)
    }
    pub(super) fn invalidate(&mut self) {
        self.reusable = false;
    }
}

impl Connection {
    fn backend_gone(&self) -> bool {
        let (Some(runtime), Some((pid, started))) = (self.runtime.as_ref(), self.identity.as_ref())
        else {
            return false;
        };
        let result = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), async {
                let (client, connection) = self.config.connect(postgres_native_tls::MakeTlsConnector::new(self.tls.as_ref().clone())).await?;
                let mut driver = tokio::task::JoinSet::new();
                driver.spawn(connection);
                let result = client.query_one("SELECT NOT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND extract(epoch FROM backend_start)::text=$2)", &[pid,started]).await;
                drop(client);
                driver.abort_all();
                result.and_then(|row|row.try_get::<_,bool>(0))
            }).await
        });
        matches!(result, Ok(Ok(true)))
    }
}
