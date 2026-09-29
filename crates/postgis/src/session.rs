//! Synchronous adapter over an owned async driver so every network wait is bounded.
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, task::JoinHandle};
use tokio_postgres::{
    Config, Row, RowStream,
    types::{BorrowToSql, ToSql},
};
use tokio_stream::StreamExt;

#[derive(Debug)]
pub(super) enum Failure {
    Config,
    Postgres(tokio_postgres::Error),
    Io(std::io::Error),
}
impl From<tokio_postgres::Error> for Failure {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Postgres(error)
    }
}
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
type Result<T> = std::result::Result<T, Failure>;
fn expired() -> Failure {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "PostGIS network deadline exceeded",
    )
    .into()
}
pub(super) struct Client {
    runtime: Option<Runtime>,
    client: tokio_postgres::Client,
    driver: RefCell<Option<JoinHandle<()>>>,
    timeout: Duration,
}
impl Client {
    pub fn connect(dsn: &str, timeout: Duration) -> Result<Self> {
        Self::connect_with_timeout(dsn, timeout, Duration::from_secs(10))
    }
    fn connect_with_timeout(
        dsn: &str,
        timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<Self> {
        let mut config: Config = dsn.parse().map_err(|_| Failure::Config)?;
        config
            .connect_timeout(connect_timeout)
            .application_name("geoledger");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let tls = native_tls::TlsConnector::new().map_err(std::io::Error::other)?;
        let result = runtime.block_on(async {
            tokio::time::timeout(
                connect_timeout,
                config.connect(postgres_native_tls::MakeTlsConnector::new(tls)),
            )
            .await
        });
        let (client, connection) = match result {
            Ok(Ok(value)) => value,
            error => {
                runtime.shutdown_background();
                return Err(match error {
                    Ok(Err(e)) => e.into(),
                    _ => expired(),
                });
            }
        };
        let driver = runtime.spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            runtime: Some(runtime),
            client,
            driver: RefCell::new(Some(driver)),
            timeout,
        })
    }
    fn run<T>(
        &self,
        deadline: Instant,
        future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
    ) -> Result<T> {
        if self.driver.borrow().is_none() {
            return Err(expired());
        }
        let runtime = self.runtime.as_ref().ok_or_else(expired)?;
        let result = if Instant::now() >= deadline {
            None
        } else {
            Some(runtime.block_on(async { tokio::time::timeout_at(deadline.into(), future).await }))
        };
        match result {
            Some(Ok(result)) => result.map_err(Into::into),
            _ => {
                // Await cancellation so sockets are gone before the repository lock is released.
                if let Some(driver) = self.driver.borrow_mut().take() {
                    driver.abort();
                    let _ = runtime.block_on(driver);
                }
                Err(expired())
            }
        }
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.run(
            Instant::now() + self.timeout,
            self.client.batch_execute(sql),
        )
    }
    pub fn execute(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64> {
        self.run(
            Instant::now() + self.timeout,
            self.client.execute(sql, params),
        )
    }
    pub fn query(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>> {
        self.run(
            Instant::now() + self.timeout,
            self.client.query(sql, params),
        )
    }
    pub fn query_one(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        self.run(
            Instant::now() + self.timeout,
            self.client.query_one(sql, params),
        )
    }
    pub fn query_opt(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>> {
        self.run(
            Instant::now() + self.timeout,
            self.client.query_opt(sql, params),
        )
    }
    pub fn query_raw<P, I>(&mut self, sql: &str, params: I) -> Result<Rows<'_>>
    where
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        let deadline = Instant::now() + self.timeout;
        let stream = self.run(deadline, self.client.query_raw(sql, params))?;
        Ok(Rows {
            client: self,
            stream: Box::pin(stream),
            deadline,
        })
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.get_mut().take() {
            driver.abort();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
pub(super) struct Rows<'a> {
    client: &'a Client,
    stream: Pin<Box<RowStream>>,
    deadline: Instant,
}
impl fallible_iterator::FallibleIterator for Rows<'_> {
    type Item = Row;
    type Error = Failure;
    fn next(&mut self) -> Result<Option<Row>> {
        self.client.run(self.deadline, async {
            self.stream.next().await.transpose()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fallible_iterator::FallibleIterator;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn stalled_network_phases_close_the_socket_and_disable_further_queries()
    -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for phase in ["ssl", "tls", "startup", "query", "commit", "stream"] {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?;
            let server = std::thread::spawn(move || -> std::io::Result<()> {
                let (mut peer, _) = listener.accept()?;
                peer.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut size = [0; 4];
                peer.read_exact(&mut size)?;
                let size = u32::from_be_bytes(size) as usize;
                assert!((8..=4096).contains(&size));
                let mut startup = vec![0; size - 4];
                peer.read_exact(&mut startup)?;
                if phase == "tls" {
                    peer.write_all(b"S")?;
                } else if ["query", "commit", "stream"].contains(&phase) {
                    peer.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")?;
                }
                if phase == "commit" || phase == "stream" {
                    loop {
                        let mut tag = [0];
                        peer.read_exact(&mut tag)?;
                        let mut length = [0; 4];
                        peer.read_exact(&mut length)?;
                        let length = u32::from_be_bytes(length) as usize;
                        assert!((4..=4096).contains(&length));
                        let mut body = vec![0; length - 4];
                        peer.read_exact(&mut body)?;
                        if phase == "commit" {
                            assert_eq!(tag[0], b'Q');
                            if body == b"BEGIN\0" {
                                peer.write_all(b"C\0\0\0\x0aBEGIN\0Z\0\0\0\x05T")?;
                            } else {
                                assert_eq!(body, b"COMMIT\0");
                                break;
                            }
                        } else {
                            match tag[0] {
                                b'P' => peer.write_all(b"1\0\0\0\x04")?,
                                b'D' => peer.write_all(b"t\0\0\0\x06\0\0T\0\0\0\x06\0\0")?,
                                b'S' => peer.write_all(b"Z\0\0\0\x05I")?,
                                b'B' => {
                                    peer.write_all(b"2\0\0\0\x04D\0\0\0\x06\0\0")?;
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let mut buffer = [0; 4096];
                loop {
                    match peer.read(&mut buffer) {
                        Ok(0) => return Ok(()),
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(e),
                    }
                }
            });
            let dsn = format!(
                "host=127.0.0.1 port={} user=stub dbname=stub sslmode={}",
                address.port(),
                if phase == "ssl" || phase == "tls" {
                    "require"
                } else {
                    "disable"
                }
            );
            let started = Instant::now();
            let connection = Client::connect_with_timeout(
                &dsn,
                Duration::from_millis(200),
                Duration::from_millis(200),
            );
            let error = if ["ssl", "tls", "startup"].contains(&phase) {
                match connection {
                    Err(e) => e,
                    Ok(_) => return Err("expected handshake timeout".into()),
                }
            } else {
                let mut client = connection.map_err(crate::pg_error)?;
                let result = match phase {
                    "commit" => {
                        client.batch_execute("BEGIN").map_err(crate::pg_error)?;
                        client.batch_execute("COMMIT")
                    }
                    "stream" => {
                        let mut rows = client
                            .query_raw("SELECT 1", std::iter::empty::<&(dyn ToSql + Sync)>())
                            .map_err(crate::pg_error)?;
                        assert!(rows.next().map_err(crate::pg_error)?.is_some());
                        rows.next().map(|_| ())
                    }
                    _ => client.query_one("SELECT 1", &[]).map(|_| ()),
                };
                let error = result.err().ok_or("expected query timeout")?;
                assert!(client.driver.borrow().is_none());
                assert!(client.batch_execute("ROLLBACK").is_err());
                error
            };
            assert!(
                matches!(error, Failure::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut),
                "{phase}: {error:?}"
            );
            assert!(started.elapsed() < Duration::from_secs(2), "{phase}");
            server.join().map_err(|_| "protocol stub panicked")??;
        }
        Ok(())
    }
}
