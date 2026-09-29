//! Volo Thrift control plane. All operations enter the shared Application service.
use crate::{MAX_REQUEST_BYTES, Service, public_message, thrift_proto::*};
use geoledger::{Application, Command};
use geoledger_core::{Error, Result};
use pilota::FastStr;
use std::net::SocketAddr;
use volo_thrift::MaybeException;

#[derive(Clone)]
pub struct Rpc {
    service: Service,
}
impl Rpc {
    pub fn new(service: Service) -> Self {
        Self { service }
    }
    async fn run(
        &self,
        auth: Option<FastStr>,
        command: impl FnOnce() -> Result<Command>,
    ) -> std::result::Result<JsonReply, ApiError> {
        if !self.service.authorized(auth.as_deref()) {
            return Err(ApiError {
                code: "unauthenticated".into(),
                message: "Bearer token required".into(),
            });
        }
        let command = command().map_err(api_error)?;
        let json = self
            .service
            .execute_json(command)
            .await
            .map_err(api_error)?;
        Ok(JsonReply { json: json.into() })
    }
}
fn api_error(e: Error) -> ApiError {
    ApiError {
        code: e.code().into(),
        message: public_message(&e).into(),
    }
}
fn option(s: FastStr) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
fn author(s: FastStr) -> String {
    if s.is_empty() {
        geoledger::default_author()
    } else {
        s.to_string()
    }
}
fn reference(s: FastStr) -> String {
    if s.is_empty() {
        "HEAD".into()
    } else {
        s.to_string()
    }
}
fn limit(n: i32) -> Result<usize> {
    if n < 0 {
        return Err(Error::Invalid("limit must be non-negative".into()));
    }
    Ok(if n == 0 { 100 } else { n as usize })
}
// Keep every declared Thrift exception typed while sharing error handling.
macro_rules! method {
    ($name:ident, $request:ty, $exception:ident, $p:ident, $body:expr) => {
        async fn $name(
            &self,
            $p: $request,
            authorization: Option<FastStr>,
        ) -> std::result::Result<MaybeException<JsonReply, $exception>, volo_thrift::ServerError> {
            Ok(
                match self
                    .run(authorization, || {
                        let size = pilota::thrift::Message::size(
                            &$p,
                            &mut pilota::thrift::binary::TBinaryProtocol::new((), true),
                        );
                        if size > MAX_REQUEST_BYTES {
                            return Err(Error::Invalid("request exceeds 4 MiB".into()));
                        }
                        $body
                    })
                    .await
                {
                    Ok(reply) => MaybeException::Ok(reply),
                    Err(error) => MaybeException::Exception($exception::Error(error)),
                },
            )
        }
    };
}
impl GeoLedger for Rpc {
    method!(execute, ExecuteRequest, GeoLedgerExecuteException, p, {
        if p.command_json.len() > MAX_REQUEST_BYTES {
            return Err(Error::Invalid("command exceeds 4 MiB".into()));
        }
        Ok(serde_json::from_str(&p.command_json)?)
    });
    method!(
        status,
        StatusRequest,
        GeoLedgerStatusException,
        p,
        Ok(Command::Status {
            limit: limit(p.limit)?
        })
    );
    method!(
        import,
        ImportRequest,
        GeoLedgerImportException,
        p,
        Ok(Command::Import {
            dataset: p.dataset.to_string(),
            schema: if p.schema.is_empty() {
                "public".into()
            } else {
                p.schema.to_string()
            },
            table: p.table.to_string(),
            author: author(p.author),
            message: option(p.message)
        })
    );
    method!(
        commit,
        CommitRequest,
        GeoLedgerCommitException,
        p,
        Ok(Command::Commit {
            message: p.message.to_string(),
            author: author(p.author)
        })
    );
    method!(
        log,
        LogRequest,
        GeoLedgerLogException,
        p,
        Ok(Command::Log {
            reference: reference(p.reference),
            limit: limit(p.limit)?
        })
    );
    method!(
        diff,
        DiffRequest,
        GeoLedgerDiffException,
        p,
        Ok(Command::Diff {
            from: option(p.from),
            to: option(p.to),
            limit: limit(p.limit)?
        })
    );
    method!(
        branch,
        BranchRequest,
        GeoLedgerBranchException,
        p,
        Ok(Command::Branch {
            name: p.name.to_string(),
            from: reference(p.from)
        })
    );
    method!(
        switch,
        ReferenceRequest,
        GeoLedgerSwitchException,
        p,
        Ok(Command::Switch {
            branch: p.reference.to_string()
        })
    );
    method!(
        merge,
        MergeRequest,
        GeoLedgerMergeException,
        p,
        Ok(Command::Merge {
            source: p.reference.to_string(),
            author: author(p.author),
            message: option(p.message)
        })
    );
    method!(
        revert,
        MergeRequest,
        GeoLedgerRevertException,
        p,
        Ok(Command::Revert {
            target: p.reference.to_string(),
            author: author(p.author),
            message: option(p.message)
        })
    );
    method!(
        reset,
        ResetRequest,
        GeoLedgerResetException,
        p,
        Ok(Command::Reset {
            target: p.target.to_string(),
            hard: p.hard
        })
    );
    method!(
        restore,
        RestoreRequest,
        GeoLedgerRestoreException,
        p,
        Ok(Command::Restore { discard: p.discard })
    );
    method!(
        r#continue,
        Empty,
        GeoLedgerContinueException,
        _p,
        Ok(Command::MergeContinue)
    );
    method!(
        abort,
        Empty,
        GeoLedgerAbortException,
        _p,
        Ok(Command::MergeAbort)
    );
    method!(
        recover,
        Empty,
        GeoLedgerRecoverException,
        _p,
        Ok(Command::Recover)
    );
}

/// Standalone listener: Volo handles SIGINT/SIGTERM and drains active connections.
pub async fn serve(
    application: Application,
    address: SocketAddr,
    token: Option<String>,
) -> Result<()> {
    // Apply the same listener policy as HTTP/gRPC.
    crate::ServerConfig {
        http: address,
        grpc: address,
        token: token.clone(),
    }
    .validate()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(thrift=%listener.local_addr()?, "geoledger Thrift listener started");
    run(
        Service::new(application, token),
        volo::net::incoming::DefaultIncoming::from(listener),
    )
    .await
}

/// Also accepts custom incoming streams, allowing orderly shutdown in embedding/tests.
pub async fn run(service: Service, incoming: impl volo::net::incoming::MakeIncoming) -> Result<()> {
    let codec = crate::thrift_codec::StrictCodec::default();
    GeoLedgerServer::new(Rpc::new(service))
        .make_codec(codec)
        .run(incoming)
        .await
        .map_err(|e| Error::Backend {
            kind: geoledger_core::BackendKind::Storage,
            message: "Thrift server failed".into(),
            source: e,
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_author_defaults_to_mapseekai_and_explicit_author_is_preserved() {
        assert_eq!(super::author(pilota::FastStr::new("")), "mapseekai");
        assert_eq!(super::author("custom-author".into()), "custom-author");
    }
}
