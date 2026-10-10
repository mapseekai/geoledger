//! Encoded-byte reservations bound concurrent buffered transfers independently of execution.
//! A reservation follows the body until completion/cancellation, not just the database query.
use crate::{Error, Result};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
const UNIT: usize = 1024;
fn units(bytes: usize) -> u32 {
    bytes.div_ceil(UNIT).max(1).min(u32::MAX as usize) as u32
}
#[derive(Clone)]
pub(crate) struct ByteBudget(Arc<Semaphore>);
impl ByteBudget {
    pub fn new(bytes: usize) -> Self {
        Self(Arc::new(Semaphore::new(units(bytes) as usize)))
    }
    pub fn reserve(&self, bytes: usize) -> Result<ByteLease> {
        self.0
            .clone()
            .try_acquire_many_owned(units(bytes))
            .map(|p| {
                ByteLease(Arc::new(LeaseState {
                    pool: self.0.clone(),
                    permit: Mutex::new(p),
                }))
            })
            .map_err(|_| Error::new(429, "transfer memory budget exhausted"))
    }
}
#[derive(Clone)]
pub(crate) struct ByteLease(Arc<LeaseState>);
struct LeaseState {
    pool: Arc<Semaphore>,
    permit: Mutex<OwnedSemaphorePermit>,
}
impl ByteLease {
    /// A high-water reservation: repeated accounting of the same result never adds twice.
    pub fn grow(&self, bytes: usize) -> Result<()> {
        let mut permit = self
            .0
            .permit
            .lock()
            .map_err(|_| Error::new(500, "transfer budget unavailable"))?;
        let extra = (units(bytes) as usize).saturating_sub(permit.num_permits());
        if extra > 0 {
            let more = self
                .0
                .pool
                .clone()
                .try_acquire_many_owned(extra as u32)
                .map_err(|_| Error::new(429, "transfer memory budget exhausted"))?;
            permit.merge(more);
        }
        Ok(())
    }
    pub fn reservation(&self) -> geoledger_engine::ResponseReservation {
        let lease = self.clone();
        Arc::new(move |bytes| lease.grow(bytes))
    }
    pub fn shrink(&self, bytes: usize) {
        if let Ok(mut permit) = self.0.permit.lock() {
            let excess = permit.num_permits().saturating_sub(units(bytes) as usize);
            drop(permit.split(excess));
        }
    }
}
/// Charge incoming data before handing it to an aggregate decoder/body collector.
pub(crate) struct BudgetedBody<B> {
    inner: Pin<Box<B>>,
    lease: ByteLease,
    bytes: usize,
    limit: usize,
    grpc: Option<GrpcFrames>,
}
#[derive(Default)]
struct GrpcFrames {
    header: [u8; 5],
    header_len: usize,
    remaining: usize,
    declared: usize,
    message_limit: usize,
}
impl GrpcFrames {
    fn accept(
        &mut self,
        mut data: &[u8],
        lease: &ByteLease,
        wire_limit: usize,
    ) -> std::result::Result<(), tonic::Status> {
        while !data.is_empty() {
            if self.remaining > 0 {
                let count = self.remaining.min(data.len());
                self.remaining -= count;
                data = &data[count..];
                continue;
            }
            let count = (5 - self.header_len).min(data.len());
            self.header[self.header_len..self.header_len + count].copy_from_slice(&data[..count]);
            self.header_len += count;
            data = &data[count..];
            if self.header_len == 5 {
                let len = u32::from_be_bytes([
                    self.header[1],
                    self.header[2],
                    self.header[3],
                    self.header[4],
                ]) as usize;
                if len > self.message_limit {
                    return Err(tonic::Status::resource_exhausted(
                        "request exceeds encoded message budget",
                    ));
                }
                self.declared = self.declared.saturating_add(5).saturating_add(len);
                if self.declared > wire_limit {
                    return Err(tonic::Status::resource_exhausted(
                        "request exceeds encoded message budget",
                    ));
                }
                // Tonic reserves the announced message size after reading this header.
                // Charge it now, even when the sender stalls before transmitting the payload.
                lease.grow(self.declared).map_err(|_| {
                    tonic::Status::resource_exhausted("receive memory budget exhausted")
                })?;
                self.remaining = len;
                self.header_len = 0;
            }
        }
        Ok(())
    }
}
impl<B> BudgetedBody<B> {
    pub fn new(body: B, lease: ByteLease, limit: usize) -> Self {
        Self {
            inner: Box::pin(body),
            lease,
            bytes: 0,
            limit,
            grpc: None,
        }
    }
    pub fn grpc(body: B, lease: ByteLease, message_limit: usize, wire_limit: usize) -> Self {
        let mut body = Self::new(body, lease, wire_limit);
        body.grpc = Some(GrpcFrames {
            message_limit,
            ..Default::default()
        });
        body
    }
}
impl<B> http_body::Body for BudgetedBody<B>
where
    B: http_body::Body<Data = axum::body::Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Data = axum::body::Bytes;
    type Error = tonic::Status;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes = this.bytes.saturating_add(data.len());
                    if this.bytes > this.limit {
                        return Poll::Ready(Some(Err(tonic::Status::resource_exhausted(
                            "request exceeds encoded message budget",
                        ))));
                    }
                    if let Some(grpc) = &mut this.grpc
                        && let Err(error) = grpc.accept(data, &this.lease, this.limit)
                    {
                        return Poll::Ready(Some(Err(error)));
                    }
                    if this.lease.grow(this.bytes).is_err() {
                        return Poll::Ready(Some(Err(tonic::Status::resource_exhausted(
                            "receive memory budget exhausted",
                        ))));
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                Poll::Ready(Some(Err(tonic::Status::from_error(error.into()))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}
pub(crate) struct LeasedBody<B> {
    inner: Pin<Box<B>>,
    lease: Option<ByteLease>,
}
impl<B> LeasedBody<B> {
    pub fn new(body: B, lease: ByteLease) -> Self {
        Self {
            inner: Box::pin(body),
            lease: Some(lease),
        }
    }
}
struct LeasedBytes {
    bytes: axum::body::Bytes,
    _lease: ByteLease,
}
impl AsRef<[u8]> for LeasedBytes {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}
impl<B: http_body::Body<Data = axum::body::Bytes>> http_body::Body for LeasedBody<B> {
    type Data = B::Data;
    type Error = B::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let lease = this.lease.clone();
                // Hyper/h2 may retain a data frame after polling body EOF. Attach
                // ownership to the Bytes too, so socket backpressure cannot detach accounting.
                Poll::Ready(Some(Ok(frame.map_data(move |bytes| match lease {
                    Some(lease) => axum::body::Bytes::from_owner(LeasedBytes {
                        bytes,
                        _lease: lease,
                    }),
                    None => bytes,
                }))))
            }
            Poll::Ready(None) => {
                this.lease = None;
                Poll::Ready(None)
            }
            other => other,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transport_data_frames_keep_reservation_after_body_eof() -> Result<()> {
        use http_body_util::BodyExt;
        let budget = ByteBudget::new(4096);
        let lease = budget.reserve(4096)?;
        let mut body = LeasedBody::new(axum::body::Body::from(vec![1u8; 3000]), lease);
        let frame = body
            .frame()
            .await
            .ok_or_else(|| Error::new(500, "missing frame"))?
            .map_err(|_| Error::new(500, "body failed"))?;
        let data = frame
            .into_data()
            .map_err(|_| Error::new(500, "missing data"))?;
        assert!(body.frame().await.is_none());
        drop(body);
        assert!(
            budget.reserve(1024).is_err(),
            "transport-owned data lost its reservation"
        );
        drop(data);
        assert!(budget.reserve(4096).is_ok());
        Ok(())
    }
    #[test]
    fn grpc_announced_lengths_are_reserved_before_payload_and_handle_split_headers() -> Result<()> {
        let budget = ByteBudget::new(4096);
        let lease = budget.reserve(0)?;
        let mut frames = GrpcFrames {
            message_limit: 4096,
            ..Default::default()
        };
        assert!(frames.accept(&[0, 0], &lease, 8192).is_ok());
        assert!(frames.accept(&[0, 12, 0], &lease, 8192).is_ok());
        // Only five bytes arrived, but the 3072-byte advertised payload is already charged.
        assert!(budget.reserve(1024).is_err());
        assert!(frames.accept(&vec![0; 3072], &lease, 8192).is_ok());
        assert!(frames.accept(&[0, 0, 0, 12, 0], &lease, 8192).is_err());
        drop(lease);
        assert!(budget.reserve(4096).is_ok());
        Ok(())
    }
    #[test]
    fn growth_is_bounded_and_failed_growth_does_not_leak() -> Result<()> {
        let budget = ByteBudget::new(8192);
        let lease = budget.reserve(0)?;
        lease.grow(4096)?;
        lease.grow(3000)?;
        let second = budget.reserve(4096)?;
        assert!(lease.grow(8192).is_err());
        drop(second);
        lease.grow(8192)?;
        drop(lease);
        assert!(budget.reserve(8192).is_ok());
        Ok(())
    }
    #[test]
    fn leases_bound_memory_and_follow_last_owner() -> Result<()> {
        let budget = ByteBudget::new(8192);
        let lease = budget.reserve(8192)?;
        let body_owner = lease.clone();
        assert!(budget.reserve(1024).is_err());
        lease.shrink(2048);
        let spare = budget.reserve(6144)?;
        drop(lease);
        assert!(budget.reserve(1024).is_err());
        drop(body_owner);
        let recovered = budget.reserve(2048)?;
        drop((spare, recovered));
        assert!(budget.reserve(8192).is_ok());
        Ok(())
    }
}
