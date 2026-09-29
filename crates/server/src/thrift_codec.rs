//! Strict ingress boundary for the locked Pilota binary decoder.
//! Pilota still contains unchecked FastStr construction; this wrapper establishes
//! its UTF-8 and length invariants rather than claiming to fix upstream code.
use pilota::{
    Bytes, LinkedBytes,
    thrift::{ProtocolExceptionKind, ThriftException},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use volo_thrift::{
    EntryMessage, ThriftMessage,
    codec::{
        Decoder, Encoder, MakeCodec,
        default::{
            MakeZeroCopyCodec, ZeroCopyDecoder, ZeroCopyEncoder,
            framed::{FramedEncoder, HasFramed},
            thrift::{MakeThriftCodec, ProtocolBinary, ThriftCodec},
        },
    },
    context::ThriftContext,
};

const OVERHEAD: usize = 64 * 1024;
const MAX_FRAME: usize = crate::MAX_REQUEST_BYTES + OVERHEAD;
const MAX_DEPTH: usize = 32;
const MAX_VALUES: usize = 65_536;
const READ_DEADLINE: Duration = Duration::from_secs(10);
const WRITE_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct StrictCodec {
    permits: Arc<Semaphore>,
}
impl Default for StrictCodec {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(8)),
        }
    }
}
pub(crate) struct StrictDecoder<R> {
    reader: R,
    // A connection owns its admission slot through all reads and decoding.
    // Idle peers expire too; buffers are local to decode, never cached.
    permit: Option<OwnedSemaphorePermit>,
}
pub(crate) struct StrictEncoder<W> {
    writer: W,
    inner: FramedEncoder<ThriftCodec>,
}
impl<R, W> MakeCodec<R, W> for StrictCodec
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    type Encoder = StrictEncoder<W>;
    type Decoder = StrictDecoder<R>;
    fn make_codec(&self, reader: R, writer: W) -> (Self::Encoder, Self::Decoder) {
        let (encoder, _) = MakeThriftCodec::default().make_codec();
        (
            StrictEncoder {
                writer,
                inner: FramedEncoder::new(encoder, (crate::MAX_RESPONSE_BYTES + OVERHEAD) as i32),
            },
            StrictDecoder {
                reader,
                permit: self.permits.clone().try_acquire_owned().ok(),
            },
        )
    }
}
fn invalid(message: &'static str) -> ThriftException {
    pilota::thrift::new_protocol_exception(ProtocolExceptionKind::InvalidData, message)
}
impl<R: AsyncRead + Unpin + Send + Sync + 'static> Decoder for StrictDecoder<R> {
    async fn decode<Msg: Send + EntryMessage, Cx: ThriftContext>(
        &mut self,
        cx: &mut Cx,
    ) -> Result<Option<ThriftMessage<Msg>>, ThriftException> {
        if self.permit.is_none() {
            return Err(invalid("Thrift ingress capacity exhausted"));
        }
        cx.stats_mut().record_read_start_at();
        let frame = tokio::time::timeout(READ_DEADLINE, read_frame(&mut self.reader))
            .await
            .map_err(|_| invalid("Thrift frame read deadline exceeded"))??;
        let Some(frame) = frame else {
            return Ok(None);
        };
        cx.stats_mut().record_read_end_at();
        cx.stats_mut().set_read_size(frame.len() + 4);
        cx.stats_mut().record_decode_start_at();
        validate(&frame)?;
        // Only validated strict binary bytes reach the synchronous codec.
        // No dependency asynchronous/unframed decode path is reachable.
        let mut bytes = Bytes::from(frame);
        cx.extensions_mut().insert(HasFramed);
        cx.extensions_mut().insert(ProtocolBinary);
        let decoded = ThriftCodec::default().decode(cx, &mut bytes);
        cx.stats_mut().record_decode_end_at();
        decoded
    }
}
async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Option<Vec<u8>>, ThriftException> {
    let mut header = [0; 4];
    if reader.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await?;
    let length = i32::from_be_bytes(header);
    if length <= 0 || length as usize > MAX_FRAME {
        return Err(invalid("invalid Thrift frame length"));
    }
    let mut frame = vec![0; length as usize];
    reader.read_exact(&mut frame).await?;
    Ok(Some(frame))
}
impl<W: AsyncWrite + Unpin + Send + Sync + 'static> Encoder for StrictEncoder<W> {
    async fn encode<Msg: Send + EntryMessage, Cx: ThriftContext>(
        &mut self,
        cx: &mut Cx,
        msg: ThriftMessage<Msg>,
    ) -> Result<(), ThriftException> {
        // Local buffer drops on every error/cancellation, including partial writes.
        cx.extensions_mut().insert(HasFramed);
        cx.extensions_mut().insert(ProtocolBinary);
        cx.stats_mut().record_encode_start_at();
        let (size, capacity) = self.inner.size(cx, &msg)?;
        cx.stats_mut().set_write_size(size);
        let mut bytes = LinkedBytes::new();
        bytes.reserve(capacity);
        self.inner.encode(cx, &mut bytes, msg)?;
        cx.stats_mut().record_encode_end_at();
        cx.stats_mut().record_write_start_at();
        write_frame(&mut self.writer, &mut bytes, WRITE_DEADLINE).await?;
        cx.stats_mut().record_write_end_at();
        Ok(())
    }
}

// A peer that stops reading must not retain an encoded response and its
// connection admission slot indefinitely.
async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    bytes: &mut LinkedBytes,
    deadline: Duration,
) -> Result<(), ThriftException> {
    tokio::time::timeout(deadline, async {
        bytes.write_all_vectored(writer).await?;
        writer.flush().await?;
        Ok::<(), ThriftException>(())
    })
    .await
    .map_err(|_| invalid("Thrift response write deadline exceeded"))?
}

// Borrows the bounded frame and never allocates. Every value consumes budget
// and bytes; recursion is bounded before entering another level.
struct Parser<'a> {
    bytes: &'a [u8],
    budget: usize,
}
#[derive(Clone, Copy)]
enum Schema<'a> {
    Args(&'a str),
    Request(&'a str),
    Unknown,
}
impl<'a> Parser<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ThriftException> {
        if n > self.bytes.len() {
            return Err(invalid("truncated Thrift value"));
        }
        let (value, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, ThriftException> {
        Ok(self.take(1)?[0])
    }
    fn length(&mut self) -> Result<usize, ThriftException> {
        let b = self.take(4)?;
        let n = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        if n < 0 {
            return Err(invalid("negative Thrift length"));
        }
        Ok(n as usize)
    }
    fn string(&mut self, utf8: bool) -> Result<&'a [u8], ThriftException> {
        let n = self.length()?;
        let b = self.take(n)?;
        if utf8 && std::str::from_utf8(b).is_err() {
            return Err(invalid("invalid Thrift UTF-8"));
        }
        Ok(b)
    }
    fn structure(&mut self, depth: usize, schema: Schema<'a>) -> Result<(), ThriftException> {
        loop {
            let t = self.byte()?;
            if t == 0 {
                return Ok(());
            }
            let b = self.take(2)?;
            let id = i16::from_be_bytes([b[0], b[1]]);
            let child = match schema {
                Schema::Args(method) if id == 1 && t == 12 => Schema::Request(method),
                _ => Schema::Unknown,
            };
            let utf8 = match schema {
                Schema::Args(_) => id == 2,
                Schema::Request(method) => string_field(method, id),
                Schema::Unknown => false,
            };
            self.value(t, depth + 1, child, utf8)?;
        }
    }
    fn value(
        &mut self,
        t: u8,
        depth: usize,
        schema: Schema<'a>,
        utf8: bool,
    ) -> Result<(), ThriftException> {
        if depth > MAX_DEPTH || self.budget == 0 {
            return Err(invalid("Thrift complexity limit exceeded"));
        }
        self.budget -= 1;
        match t {
            2 | 3 => {
                self.take(1)?;
            }
            6 => {
                self.take(2)?;
            }
            8 => {
                self.take(4)?;
            }
            4 | 10 => {
                self.take(8)?;
            }
            16 => {
                self.take(16)?;
            }
            11 => {
                self.string(utf8)?;
            }
            12 => self.structure(depth, schema)?,
            13..=15 => {
                let first = self.byte()?;
                let second = if t == 13 { Some(self.byte()?) } else { None };
                let count = self.length()?;
                let min = minimum_size(first)? + second.map(minimum_size).transpose()?.unwrap_or(0);
                let values = count
                    .checked_mul(if second.is_some() { 2 } else { 1 })
                    .ok_or_else(|| invalid("Thrift collection overflow"))?;
                if values > self.budget || count > self.bytes.len() / min {
                    return Err(invalid("Thrift collection limit exceeded"));
                }
                for _ in 0..count {
                    self.value(first, depth + 1, Schema::Unknown, false)?;
                    if let Some(second) = second {
                        self.value(second, depth + 1, Schema::Unknown, false)?;
                    }
                }
            }
            _ => return Err(invalid("invalid Thrift value type")),
        }
        Ok(())
    }
}
fn minimum_size(t: u8) -> Result<usize, ThriftException> {
    match t {
        2 | 3 | 12 => Ok(1),
        6 => Ok(2),
        8 | 11 => Ok(4),
        4 | 10 => Ok(8),
        16 => Ok(16),
        13 => Ok(6),
        14 | 15 => Ok(5),
        _ => Err(invalid("invalid Thrift collection type")),
    }
}
// Matches the unchanged v1 IDL. Unknown binary fields are skipped as bytes by
// Pilota, and need not contain UTF-8.
fn string_field(method: &str, id: i16) -> bool {
    match method {
        "execute" | "log" | "switch" | "reset" => id == 1,
        "commit" | "branch" | "diff" => (1..=2).contains(&id),
        "merge" | "revert" => (1..=3).contains(&id),
        "import" => (1..=5).contains(&id),
        _ => false,
    }
}
fn validate(frame: &[u8]) -> Result<(), ThriftException> {
    if frame.len() > MAX_FRAME {
        return Err(invalid("Thrift frame too large"));
    }
    let mut p = Parser {
        bytes: frame,
        budget: MAX_VALUES,
    };
    if p.take(4)? != [0x80, 1, 0, 1] {
        return Err(invalid("expected strict binary Thrift call"));
    }
    let method =
        std::str::from_utf8(p.string(true)?).map_err(|_| invalid("invalid method UTF-8"))?;
    p.take(4)?;
    p.structure(0, Schema::Args(method))?;
    if !p.bytes.is_empty() {
        return Err(invalid("trailing Thrift frame bytes"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::thrift_proto::GeoLedgerRequestRecv;
    use volo_thrift::context::ServerContext;

    #[tokio::test]
    async fn stalled_response_write_expires() {
        let (mut writer, _unread_peer) = tokio::io::duplex(1);
        let mut bytes = LinkedBytes::new();
        bytes.insert(Bytes::from_static(&[1u8; 32]));
        let result = write_frame(&mut writer, &mut bytes, Duration::from_millis(20)).await;
        assert!(result.is_err());
    }

    #[test]
    fn idl_changes_require_reviewing_the_utf8_field_map() {
        // Keep schema-aware validation in lockstep with generated decoding.
        // An IDL edit must deliberately update the validator and this digest.
        let idl = include_str!("../../thrift-gen/idl/geoledger.thrift");
        let digest = geoledger_core::object::digest("thrift-ingress-schema", idl.as_bytes());
        assert_eq!(
            digest.as_str(),
            "c438b19b8792b03c615a1d0502e91a607a1468095aee9aa9f72f1791b069302f"
        );
    }

    fn call(method: &[u8], fields: &[u8]) -> Vec<u8> {
        let mut b = vec![0x80, 1, 0, 1];
        b.extend_from_slice(&(method.len() as i32).to_be_bytes());
        b.extend_from_slice(method);
        b.extend_from_slice(&1_i32.to_be_bytes());
        b.extend_from_slice(fields);
        b.push(0);
        b
    }
    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut b = (payload.len() as i32).to_be_bytes().to_vec();
        b.extend_from_slice(payload);
        b
    }
    fn status() -> Vec<u8> {
        call(b"status", &[12, 0, 1, 8, 0, 1, 0, 0, 0, 1, 0])
    }

    #[test]
    fn validates_strings_types_lengths_and_recursion_without_allocation() {
        assert!(validate(&status()).is_ok());
        assert!(validate(&call(&[0xff], &[])).is_err());
        assert!(validate(&call(b"status", &[11, 0, 2, 0, 0, 0, 1, 0xff])).is_err());
        for (method, strings) in [
            ("execute", 1),
            ("import", 5),
            ("commit", 2),
            ("log", 1),
            ("diff", 2),
            ("branch", 2),
            ("switch", 1),
            ("merge", 3),
            ("revert", 3),
            ("reset", 1),
        ] {
            for id in 1..=strings {
                let fields = [12, 0, 1, 11, 0, id as u8, 0, 0, 0, 1, 0xff, 0];
                assert!(
                    validate(&call(method.as_bytes(), &fields)).is_err(),
                    "{method} field {id}"
                );
            }
        }
        // Unknown binary fields are not declared strings, including in containers.
        assert!(validate(&call(b"status", &[11, 0, 99, 0, 0, 0, 1, 0xff])).is_ok());
        assert!(
            validate(&call(
                b"status",
                &[15, 0, 99, 11, 0, 0, 0, 1, 0, 0, 0, 1, 0xff]
            ))
            .is_ok()
        );
        for n in [-1_i32, i32::MAX, 100] {
            let mut fields = vec![11, 0, 99];
            fields.extend_from_slice(&n.to_be_bytes());
            assert!(validate(&call(b"status", &fields)).is_err());
            for t in [13, 14, 15] {
                let mut fields = vec![t, 0, 99, 11];
                if t == 13 {
                    fields.push(11);
                }
                fields.extend_from_slice(&n.to_be_bytes());
                assert!(validate(&call(b"status", &fields)).is_err());
            }
        }
        let mut fields = [12, 0, 99].repeat(MAX_DEPTH + 1);
        fields.extend(std::iter::repeat_n(0, MAX_DEPTH + 1));
        assert!(validate(&call(b"status", &fields)).is_err());
        let mut fields = vec![15, 0, 99, 2];
        fields.extend_from_slice(&((MAX_VALUES + 1) as i32).to_be_bytes());
        fields.extend(std::iter::repeat_n(0, MAX_VALUES + 1));
        assert!(validate(&call(b"status", &fields)).is_err());
        for length in 0..status().len() {
            assert!(validate(&status()[..length]).is_err());
        }
    }

    #[tokio::test]
    async fn strict_decoder_rejects_unframed_headers_and_preserves_binary_extensions() {
        let codec = StrictCodec::default();
        let mut binary = status();
        binary.pop();
        binary.extend_from_slice(&[11, 0, 99, 0, 0, 0, 1, 0xff, 0]);
        for payload in [status(), binary] {
            let (_, mut decoder) =
                codec.make_codec(std::io::Cursor::new(framed(&payload)), tokio::io::sink());
            assert!(
                decoder
                    .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        for input in [
            status(),
            (-1_i32).to_be_bytes().to_vec(),
            i32::MAX.to_be_bytes().to_vec(),
            ((MAX_FRAME + 1) as i32).to_be_bytes().to_vec(),
            framed(&call(&[0xff], &[])),
            framed(&call(b"status", &[11, 0, 2, 0, 0, 0, 1, 0xff])),
        ] {
            let (_, mut decoder) = codec.make_codec(std::io::Cursor::new(input), tokio::io::sink());
            assert!(
                decoder
                    .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn deterministic_mutations_exercise_real_decoder_after_validation() {
        let seed = status();
        let mut random = 0x12345678_u32;
        for _ in 0..2048 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let mut input = seed.clone();
            input[random as usize % seed.len()] = (random >> 16) as u8;
            // This invokes generated code only when the same ingress validator
            // accepts the mutated frame. Panics fail the test naturally.
            let (_, mut decoder) = StrictCodec::default()
                .make_codec(std::io::Cursor::new(framed(&input)), tokio::io::sink());
            let _ = decoder
                .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                .await;
        }
    }

    #[tokio::test]
    async fn framed_generated_messages_roundtrip_and_authenticate_over_duplex() {
        use crate::thrift_proto::*;
        use pilota::thrift::TMessageType;
        use volo::context::{Role, RpcInfo};
        use volo_thrift::{MaybeException, context::ClientContext};
        let directory = tempfile::tempdir().unwrap();
        let rpc = crate::thrift::Rpc::new(crate::Service::new(
            geoledger::Application::new(directory.path()),
            Some("test-token".into()),
        ));
        let (client, server) = tokio::io::duplex(8192);
        let (mut client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let (mut server_encoder, mut server_decoder) =
            StrictCodec::default().make_codec(server_read, server_write);
        let (mut client_encoder, _) =
            StrictCodec::default().make_codec(tokio::io::empty(), client_write);
        for authorization in [None, Some("Bearer test-token".into())] {
            let mut info = RpcInfo::with_role(Role::Client);
            info.set_method("execute".into());
            let mut client_cx = ClientContext::new(1, info, TMessageType::Call);
            let request = GeoLedgerRequestSend::Execute(GeoLedgerExecuteArgsSend {
                request: ExecuteRequest {
                    command_json: r#"{"op":"init"}"#.into(),
                },
                authorization: authorization.clone(),
            });
            let message = ThriftMessage::mk_client_msg(&client_cx, request);
            client_encoder
                .encode(&mut client_cx, message)
                .await
                .unwrap();
            let mut server_cx = ServerContext::default();
            let message = server_decoder
                .decode::<GeoLedgerRequestRecv, _>(&mut server_cx)
                .await
                .unwrap()
                .unwrap();
            let GeoLedgerRequestRecv::Execute(args) = message.data.unwrap() else {
                panic!("wrong method");
            };
            let reply = rpc.execute(args.request, args.authorization).await.unwrap();
            let reply = match reply {
                MaybeException::Ok(reply) => {
                    assert!(authorization.is_some());
                    GeoLedgerExecuteResultSend::Ok(reply)
                }
                MaybeException::Exception(GeoLedgerExecuteException::Error(error)) => {
                    assert!(authorization.is_none());
                    assert_eq!(error.code, "unauthenticated");
                    assert!(!directory.path().join(".geoledger").exists());
                    GeoLedgerExecuteResultSend::Error(error)
                }
            };
            let message = ThriftMessage::mk_server_resp(
                &server_cx,
                Ok(GeoLedgerResponseSend::Execute(reply)),
            );
            server_encoder
                .encode(&mut server_cx, message)
                .await
                .unwrap();
            let mut response = Bytes::from(read_frame(&mut client_read).await.unwrap().unwrap());
            // Use the dependency client decoder on trusted server output to
            // verify actual wire interoperability, not only parser acceptance.
            let decoded = ThriftCodec::default()
                .decode::<GeoLedgerResponseRecv, _>(&mut client_cx, &mut response)
                .unwrap()
                .unwrap();
            assert!(matches!(
                decoded.data.unwrap(),
                GeoLedgerResponseRecv::Execute(_)
            ));
        }
        assert!(directory.path().join(".geoledger").exists());
    }

    #[tokio::test]
    async fn admission_is_bounded_and_drop_releases_slots() {
        let codec = StrictCodec::default();
        let mut held = Vec::new();
        for _ in 0..8 {
            held.push(codec.make_codec(tokio::io::empty(), tokio::io::sink()).1);
        }
        let (_, mut rejected) = codec.make_codec(tokio::io::empty(), tokio::io::sink());
        assert!(
            rejected
                .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                .await
                .is_err()
        );
        held.pop();
        let (_, mut accepted) =
            codec.make_codec(std::io::Cursor::new(framed(&status())), tokio::io::sink());
        assert!(
            accepted
                .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn partial_frame_deadline_and_cancellation_release_admission() {
        let codec = StrictCodec::default();
        let (reader, mut peer) = tokio::io::duplex(16);
        peer.write_all(&100_i32.to_be_bytes()).await.unwrap();
        let (_, mut decoder) = codec.make_codec(reader, tokio::io::sink());
        let err = decoder
            .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("deadline"));
        drop(decoder);
        assert_eq!(codec.permits.available_permits(), 8);
        let (reader, _peer) = tokio::io::duplex(16);
        let (_, mut decoder) = codec.make_codec(reader, tokio::io::sink());
        let task = tokio::spawn(async move {
            decoder
                .decode::<GeoLedgerRequestRecv, _>(&mut ServerContext::default())
                .await
        });
        task.abort();
        let _ = task.await;
        assert_eq!(codec.permits.available_permits(), 8);
    }
}
