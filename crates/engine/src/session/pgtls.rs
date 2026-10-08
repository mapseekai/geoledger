//! libpq-compatible TLS policy for PostgreSQL connections.
//!
//! tokio-postgres itself only understands `sslmode=disable|prefer|require` and never
//! verifies certificates. GeoLedger strips the libpq TLS parameters from the DSN and
//! builds the native-tls connector itself:
//!
//! | sslmode | encryption | chain verified | hostname verified |
//! |---|---|---|---|
//! | disable | no | – | – |
//! | require | yes, mandatory | only when `sslrootcert` is given (libpq rule) | no |
//! | verify-ca | yes, mandatory | yes | no |
//! | verify-full (default) | yes, mandatory | yes | yes |
//!
//! `prefer` and `allow` are rejected because they silently fall back to plaintext.
//! `sslrootcert` (PEM bundle, or `system`), `sslcert` and `sslkey` (PKCS#8 PEM) are supported.
use crate::{Error, Result};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Require,
    VerifyCa,
    VerifyFull,
}
impl SslMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}
/// Parsed connection settings. Never Debug: the DSN contains credentials.
pub struct PgSettings {
    pub config: tokio_postgres::Config,
    pub mode: SslMode,
    root_cert: Option<PathBuf>,
    verify_chain: bool,
    client_cert: Option<(PathBuf, PathBuf)>,
}
/// Summary that is safe to log or validate (no secrets).
#[derive(Clone, Debug)]
pub struct PgTlsSummary {
    pub mode: SslMode,
    /// True when every configured host is a loopback address or a Unix socket.
    pub loopback_only: bool,
}
fn invalid(message: &str) -> Error {
    Error::new(400, message)
}
fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s
                    .get(i + 1..i + 3)
                    .ok_or_else(|| invalid("invalid percent-encoding in database URL"))?;
                out.push(
                    u8::from_str_radix(hex, 16)
                        .map_err(|_| invalid("invalid percent-encoding in database URL"))?,
                );
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| invalid("invalid database URL encoding"))
}
/// Split libpq TLS parameters from a URL or key=value DSN. Returns the remaining DSN.
fn split(dsn: &str) -> Result<(String, Vec<(String, String)>)> {
    const OURS: [&str; 4] = ["sslmode", "sslrootcert", "sslcert", "sslkey"];
    let mut taken = Vec::new();
    if dsn.starts_with("postgres://") || dsn.starts_with("postgresql://") {
        let Some((base, query)) = dsn.split_once('?') else {
            return Ok((dsn.to_owned(), taken));
        };
        let mut kept = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let key = percent_decode(k)?;
            if OURS.contains(&key.as_str()) {
                taken.push((key, percent_decode(v)?));
            } else {
                kept.push(pair);
            }
        }
        let rest = if kept.is_empty() {
            base.to_owned()
        } else {
            format!("{base}?{}", kept.join("&"))
        };
        return Ok((rest, taken));
    }
    // key = value pairs; values may be single-quoted with backslash escapes.
    let chars: Vec<char> = dsn.chars().collect();
    let mut i = 0;
    let mut kept = Vec::new();
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut key = String::new();
        while i < chars.len() && chars[i] != '=' && !chars[i].is_whitespace() {
            key.push(chars[i]);
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '=' {
            return Err(invalid("invalid database connection string"));
        }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < chars.len() && chars[i] == '\'' {
            i += 1;
            loop {
                match chars.get(i) {
                    None => {
                        return Err(invalid("unterminated quote in database connection string"));
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        value.push(*chars.get(i + 1).ok_or_else(|| {
                            invalid("invalid escape in database connection string")
                        })?);
                        i += 2;
                    }
                    Some(c) => {
                        value.push(*c);
                        i += 1;
                    }
                }
            }
        } else {
            while i < chars.len() && !chars[i].is_whitespace() {
                value.push(chars[i]);
                i += 1;
            }
        }
        if OURS.contains(&key.as_str()) {
            taken.push((key, value));
        } else {
            let quoted = value.replace('\\', "\\\\").replace('\'', "\\'");
            kept.push(format!("{key}='{quoted}'"));
        }
    }
    Ok((kept.join(" "), taken))
}
fn loopback(config: &tokio_postgres::Config) -> bool {
    use tokio_postgres::config::Host;
    let hosts = config.get_hosts();
    !hosts.is_empty()
        && hosts.iter().all(|h| match h {
            #[cfg(unix)]
            Host::Unix(_) => true,
            Host::Tcp(name) => {
                name == "localhost"
                    || name
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
        })
}
impl PgSettings {
    pub fn parse(dsn: &str) -> Result<Self> {
        let (rest, taken) = split(dsn)?;
        let mut mode = SslMode::VerifyFull;
        let mut root_cert = None;
        let (mut cert, mut key) = (None, None);
        for (k, v) in taken {
            match k.as_str() {
                "sslmode" => {
                    mode = match v.as_str() {
                        "disable" => SslMode::Disable,
                        "require" => SslMode::Require,
                        "verify-ca" => SslMode::VerifyCa,
                        "verify-full" => SslMode::VerifyFull,
                        "prefer" | "allow" => {
                            return Err(invalid(
                                "sslmode=prefer/allow may silently use plaintext; use verify-full, verify-ca, require or an explicit disable",
                            ));
                        }
                        _ => return Err(invalid("unsupported sslmode")),
                    }
                }
                "sslrootcert" => root_cert = Some(v),
                "sslcert" => cert = Some(PathBuf::from(v)),
                "sslkey" => key = Some(PathBuf::from(v)),
                _ => {}
            }
        }
        let client_cert = match (cert, key) {
            (Some(c), Some(k)) => Some((c, k)),
            (None, None) => None,
            _ => return Err(invalid("sslcert and sslkey must be configured together")),
        };
        let mut config: tokio_postgres::Config = rest
            .parse()
            .map_err(|_| invalid("invalid database connection string"))?;
        config.ssl_mode(if mode == SslMode::Disable {
            tokio_postgres::config::SslMode::Disable
        } else {
            // Require: the driver fails instead of falling back when TLS is refused.
            tokio_postgres::config::SslMode::Require
        });
        let verify_chain = mode != SslMode::Require || root_cert.is_some();
        let root_cert = root_cert.filter(|r| r != "system").map(PathBuf::from);
        Ok(Self {
            config,
            mode,
            root_cert,
            verify_chain,
            client_cert,
        })
    }
    pub fn summary(&self) -> PgTlsSummary {
        PgTlsSummary {
            mode: self.mode,
            loopback_only: loopback(&self.config),
        }
    }
    pub fn connector(&self) -> Result<native_tls::TlsConnector> {
        let fail = |e: &dyn std::fmt::Display| {
            Error::new(503, "database TLS configuration failed")
                .caused_by(std::io::Error::other(e.to_string()))
        };
        let mut builder = native_tls::TlsConnector::builder();
        builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
        if let Some(path) = &self.root_cert {
            let pem = std::fs::read(path).map_err(|e| fail(&e))?;
            let mut count = 0;
            for block in pem_blocks(&pem, "CERTIFICATE") {
                builder.add_root_certificate(
                    native_tls::Certificate::from_pem(&block).map_err(|e| fail(&e))?,
                );
                count += 1;
            }
            if count == 0 {
                return Err(fail(&"sslrootcert contains no certificates"));
            }
            builder.disable_built_in_roots(true);
        }
        if let Some((cert, key)) = &self.client_cert {
            let cert = std::fs::read(cert).map_err(|e| fail(&e))?;
            let key = std::fs::read(key).map_err(|e| fail(&e))?;
            builder.identity(native_tls::Identity::from_pkcs8(&cert, &key).map_err(|e| fail(&e))?);
        }
        builder.danger_accept_invalid_certs(!self.verify_chain);
        builder.danger_accept_invalid_hostnames(self.mode != SslMode::VerifyFull);
        builder.build().map_err(|e| fail(&e))
    }
}
fn pem_blocks(pem: &[u8], label: &str) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(pem);
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut out = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find(&begin) {
        let Some(stop) = rest[start..].find(&end) else {
            break;
        };
        let block = &rest[start..start + stop + end.len()];
        out.push(format!("{block}\n").into_bytes());
        rest = &rest[start + stop + end.len()..];
    }
    out
}
/// Validate a PostGIS DSN without connecting; safe to log the result.
pub fn summarize(dsn: &str) -> Result<PgTlsSummary> {
    PgSettings::parse(dsn).map(|s| s.summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_defaults_and_parameter_stripping() -> Result<()> {
        let s = PgSettings::parse("postgres://u:p@db.example:5432/gl")?;
        assert_eq!(s.mode, SslMode::VerifyFull);
        assert!(!s.summary().loopback_only);
        let s = PgSettings::parse(
            "postgresql://u@127.0.0.1/gl?application_name=x&sslmode=verify-ca&sslrootcert=%2Ftmp%2Fca.pem",
        )?;
        assert_eq!(s.mode, SslMode::VerifyCa);
        assert_eq!(
            s.root_cert.as_deref(),
            Some(std::path::Path::new("/tmp/ca.pem"))
        );
        assert!(s.summary().loopback_only);
        assert_eq!(s.config.get_application_name(), Some("x"));
        let s = PgSettings::parse(
            "host=localhost dbname=gl user='a b' sslmode=require sslrootcert=system",
        )?;
        assert_eq!(s.mode, SslMode::Require);
        assert!(s.root_cert.is_none());
        assert!(s.verify_chain);
        assert!(!PgSettings::parse("host=localhost sslmode=require")?.verify_chain);
        assert_eq!(s.config.get_user(), Some("a b"));
        assert!(
            PgSettings::parse("host=localhost sslmode=disable")?
                .summary()
                .loopback_only
        );
        for bad in [
            "postgres://h/db?sslmode=prefer",
            "host=h sslmode=allow",
            "host=h sslmode=bogus",
            "host=h sslcert=/a",
            "host=h user='unterminated",
        ] {
            assert!(PgSettings::parse(bad).is_err(), "{bad}");
        }
        Ok(())
    }
}
