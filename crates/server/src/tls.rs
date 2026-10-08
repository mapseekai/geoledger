//! Optional in-process TLS for both listeners. Certificates, keys and the optional
//! client CA are re-read on reload so rotated files take effect without a restart;
//! a failed reload keeps serving the last valid configuration.
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::WebPkiClientVerifier,
};
use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// When set, every client must present a certificate signed by this CA (mTLS).
    pub client_ca: Option<PathBuf>,
}

/// Shared, reloadable server configuration. HTTP offers h2 and http/1.1, gRPC only h2.
pub struct Tls {
    files: TlsFiles,
    http: RwLock<TlsAcceptor>,
    grpc: RwLock<TlsAcceptor>,
}

fn certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, BoxError> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("TLS certificate {}: {e}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("TLS certificate {}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!(
            "TLS certificate {} contains no certificates",
            path.display()
        )
        .into());
    }
    Ok(certs)
}

fn config(files: &TlsFiles, alpn: &[&[u8]]) -> Result<ServerConfig, BoxError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let chain = certificates(&files.cert)?;
    let key = PrivateKeyDer::from_pem_file(&files.key)
        .map_err(|e| format!("TLS private key {}: {e}", files.key.display()))?;
    let builder = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?;
    let builder = match &files.client_ca {
        Some(ca) => {
            let mut roots = RootCertStore::empty();
            for cert in certificates(ca)? {
                roots.add(cert)?;
            }
            builder.with_client_cert_verifier(
                WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build()?,
            )
        }
        None => builder.with_no_client_auth(),
    };
    let mut config = builder.with_single_cert(chain, key)?;
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(config)
}

impl Tls {
    pub fn load(files: TlsFiles) -> Result<Arc<Self>, BoxError> {
        let http = TlsAcceptor::from(Arc::new(config(&files, &[b"h2", b"http/1.1"])?));
        let grpc = TlsAcceptor::from(Arc::new(config(&files, &[b"h2"])?));
        Ok(Arc::new(Self {
            files,
            http: RwLock::new(http),
            grpc: RwLock::new(grpc),
        }))
    }
    pub fn files(&self) -> &TlsFiles {
        &self.files
    }
    /// Re-read certificate, key and client CA. On error the previous configuration stays active.
    pub fn reload(&self) -> Result<(), BoxError> {
        let http = TlsAcceptor::from(Arc::new(config(&self.files, &[b"h2", b"http/1.1"])?));
        let grpc = TlsAcceptor::from(Arc::new(config(&self.files, &[b"h2"])?));
        *self.http.write().map_err(|_| "TLS state poisoned")? = http;
        *self.grpc.write().map_err(|_| "TLS state poisoned")? = grpc;
        Ok(())
    }
    fn acceptor(&self, grpc: bool) -> Option<TlsAcceptor> {
        let lock = if grpc { &self.grpc } else { &self.http };
        lock.read().ok().map(|a| a.clone())
    }
}

/// Accepts TCP connections and completes TLS handshakes off the accept loop, so a slow or
/// malicious client cannot stall other connections. Failed handshakes are dropped quietly.
pub fn spawn_acceptor(
    listener: TcpListener,
    tls: Arc<Tls>,
    grpc: bool,
) -> tokio::sync::mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let pending = Arc::new(tokio::sync::Semaphore::new(64));
    tokio::spawn(async move {
        loop {
            let permit = tokio::select! {
                _ = tx.closed() => return,
                permit = pending.clone().acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => return,
                },
            };
            let accepted = tokio::select! {
                _ = tx.closed() => return,
                accepted = listener.accept() => accepted,
            };
            let (stream, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    // EMFILE and similar resource errors: back off instead of spinning.
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            if tx.is_closed() {
                return;
            }
            let Some(acceptor) = tls.acceptor(grpc) else {
                continue;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let result = tokio::select! {
                    _ = tx.closed() => return,
                    result = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)) => result,
                };
                match result {
                    Ok(Ok(stream)) => {
                        let _ = tx.send((stream, peer)).await;
                    }
                    Ok(Err(error)) => tracing::debug!(%peer, %error, "TLS handshake failed"),
                    Err(_) => tracing::debug!(%peer, "TLS handshake timed out"),
                }
            });
        }
    });
    rx
}

/// axum listener over completed TLS handshakes.
pub struct TlsListener {
    pub(crate) rx: tokio::sync::mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    pub(crate) local: SocketAddr,
}
impl TlsListener {
    pub fn new(listener: TcpListener, tls: Arc<Tls>) -> io::Result<Self> {
        let local = listener.local_addr()?;
        Ok(Self {
            rx: spawn_acceptor(listener, tls, false),
            local,
        })
    }
}
impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if let Some(accepted) = self.rx.recv().await {
                return accepted;
            }
            // The acceptor task never exits while the listener lives; stay pending if it did.
            std::future::pending::<()>().await;
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Client address recorded for access logs and per-IP limits, for TLS and plain listeners.
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TlsListener>>
    for PeerAddr
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TcpListener>>
    for PeerAddr
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dropping_receiver_releases_listener_and_stalled_handshakes() -> Result<(), BoxError> {
        use tokio::io::AsyncReadExt;
        let dir = tempfile::tempdir()?;
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let files = TlsFiles {
            cert: dir.path().join("cert.pem"),
            key: dir.path().join("key.pem"),
            client_ca: None,
        };
        std::fs::write(&files.cert, cert.cert.pem())?;
        std::fs::write(&files.key, cert.key_pair.serialize_pem())?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let rx = spawn_acceptor(listener, Tls::load(files)?, false);
        let mut stalled = TcpStream::connect(address).await?;
        tokio::task::yield_now().await;
        drop(rx);
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut byte = [0];
            match stalled.read(&mut byte).await {
                Ok(0) | Err(_) => (),
                _ => panic!("unexpected TLS data"),
            }
            loop {
                if let Ok(listener) = TcpListener::bind(address).await {
                    drop(listener);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }
}
