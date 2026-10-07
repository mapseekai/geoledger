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

const CAPACITY: usize = 16;
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
            geometry_cache: HashMap::new(),
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
            let config: tokio_postgres::Config = dsn.parse().map_err(Error::database)?;
            let tls = native_tls::TlsConnector::new()
                .map_err(|source| Error::new(503, "TLS configuration failed").caused_by(source))?;
            let connected = runtime.block_on(async {
                tokio::time::timeout_at(
                    deadline.into(),
                    config.connect(postgres_native_tls::MakeTlsConnector::new(tls)),
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
            session.batch_execute("SET statement_timeout='30s'; SET lock_timeout='10s'; SET search_path=pg_catalog,public")?;
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
    pub geometry_cache: HashMap<String, serde_json::Value>,
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
    pub fn query_one(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        let mut rows = self.query(sql, params)?;
        if rows.len() != 1 {
            return Err(Error::new(500, "unexpected database result"));
        }
        Ok(rows.remove(0))
    }
    pub fn query_opt(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>> {
        let mut rows = self.query(sql, params)?;
        if rows.len() > 1 {
            return Err(Error::new(500, "unexpected database result"));
        }
        Ok(rows.pop())
    }
    pub fn execute(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64> {
        let statement = self.statement(sql)?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.execute(&statement, params))
                .await
        });
        self.finish(result)
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.check_deadline()?;
        let c = self.connection.as_ref().ok_or_else(expired)?;
        let result = c.runtime.as_ref().ok_or_else(expired)?.block_on(async {
            tokio::time::timeout_at(self.deadline.into(), c.client.batch_execute(sql)).await
        });
        self.finish(result)
    }
    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.batch_execute("BEGIN")?;
        self.reusable = false;
        Ok(Transaction(self))
    }
}
/// Trusted in-process host access to the application's deadline-bound transaction.
/// Hosts must not issue transaction-control SQL or independently modify Center history.
pub struct Transaction<'a>(&'a mut Client);
impl Transaction<'_> {
    pub fn query(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>> {
        self.0.query(sql, params)
    }
    pub fn query_one(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        self.0.query_one(sql, params)
    }
    pub fn query_opt(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>> {
        self.0.query_opt(sql, params)
    }
    pub fn execute(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64> {
        self.0.execute(sql, params)
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.0.batch_execute(sql)
    }
    pub(crate) fn geometry_cache(&mut self) -> &mut HashMap<String, serde_json::Value> {
        &mut self.0.geometry_cache
    }
    pub(crate) fn commit(self) -> Result<()> {
        self.0.batch_execute("COMMIT")?;
        self.0.reusable = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_query_and_commit_drop_the_driver_and_release_the_pool_slot()
    -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for commit in [false, true] {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let (transport, mut peer) = tokio::io::duplex(4096);
            let server = std::thread::spawn(move || -> std::io::Result<bool> {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(async {
                        let length = peer.read_u32().await?;
                        let mut startup = vec![0; length as usize - 4];
                        peer.read_exact(&mut startup).await?;
                        peer.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I").await?;
                        let mut stalled = false;
                        loop {
                            let tag = match peer.read_u8().await {
                                Ok(tag) => tag,
                                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                                    return Ok(stalled);
                                }
                                Err(error) => return Err(error),
                            };
                            let length = peer.read_u32().await?;
                            let mut body = vec![0; length as usize - 4];
                            peer.read_exact(&mut body).await?;
                            if tag == b'Q' && body == b"BEGIN\0" {
                                peer.write_all(b"C\0\0\0\x0aBEGIN\0Z\0\0\0\x05T").await?;
                            } else {
                                stalled |= if commit {
                                    tag == b'Q' && body == b"COMMIT\0"
                                } else {
                                    tag == b'P'
                                };
                            }
                        }
                    })
            });
            let mut config = tokio_postgres::Config::new();
            config
                .user("stub")
                .dbname("stub")
                .ssl_mode(tokio_postgres::config::SslMode::Disable);
            let (client, connection) =
                runtime.block_on(config.connect_raw(transport, tokio_postgres::NoTls))?;
            let driver = runtime.spawn(async move {
                let _ = connection.await;
            });
            let pool = Arc::new(Pool::default());
            pool.state.lock().map_err(|_| "test mutex")?.active = 1;
            let mut session = Client {
                pool: pool.clone(),
                connection: Some(Connection {
                    runtime: Some(runtime),
                    client,
                    driver,
                    statements: HashMap::new(),
                }),
                deadline: Instant::now() + Duration::from_millis(50),
                reusable: true,
                geometry_cache: HashMap::new(),
            };
            let result = if commit {
                session.transaction()?.commit()
            } else {
                session.query_one("SELECT 1", &[]).map(|_| ())
            };
            assert_eq!(result.err().ok_or("expected deadline")?.status, 504);
            assert!(session.connection.is_none());
            drop(session);
            assert_eq!(pool.state.lock().map_err(|_| "test mutex")?.active, 0);
            assert!(server.join().map_err(|_| "stub panicked")??);
        }
        Ok(())
    }
    #[test]
    fn pool_wait_is_inside_deadline_and_does_not_consume_capacity() -> Result<()> {
        let pool = Arc::new(Pool::default());
        pool.state
            .lock()
            .map_err(|_| Error::new(500, "test mutex"))?
            .active = CAPACITY;
        let start = Instant::now();
        let error = match pool.connect("invalid dsn", Duration::from_millis(20)) {
            Err(error) => error,
            Ok(_) => return Err(Error::new(500, "expected pool timeout")),
        };
        assert_eq!(error.status, 504);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            pool.state
                .lock()
                .map_err(|_| Error::new(500, "test mutex"))?
                .active,
            CAPACITY
        );
        Ok(())
    }
}
