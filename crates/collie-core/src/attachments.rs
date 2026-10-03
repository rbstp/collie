use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use protocol::{
    AttachmentAbortParams, AttachmentBeginParams, AttachmentChunkParams, AttachmentCommitParams,
    AttachmentName, ChunkData, Request, Response, Sha256Hex, UploadId, limits,
};
use sha2::{Digest, Sha256};

use crate::session::unexpected;
use crate::{CoreError, expect_ok, invalid, new_op_id};

/// collied limits chunks per peer; a fast link backs off instead of failing.
const RATE_LIMIT_RETRIES: usize = 8;
const RATE_LIMIT_PAUSE: Duration = Duration::from_millis(250);

/// Called from a collie-core thread after each acknowledged chunk.
#[uniffi::export(callback_interface)]
pub trait UploadProgress: Send + Sync {
    fn on_progress(&self, sent: u64, total: u64);
}

pub(crate) fn check(name: &str, data: &[u8]) -> Result<AttachmentName, CoreError> {
    let name = AttachmentName::new(name.trim()).map_err(|_| {
        invalid(
            "name",
            "file name must be 1 to 64 characters, without slashes or control characters",
        )
    })?;
    if data.is_empty() {
        return Err(invalid("data", "the file is empty"));
    }
    if data.len() as u64 > limits::MAX_ATTACHMENT_BYTES {
        return Err(CoreError::TooLarge {
            message: format!(
                "the file is {:.1} MB, attachments are limited to {} MB",
                data.len() as f64 / 1_048_576.0,
                limits::MAX_ATTACHMENT_BYTES >> 20
            ),
        });
    }
    Ok(name)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// begin, sequential chunks (each awaited), commit. On any failure or on `cancel`, the
/// upload is aborted best effort. Nothing resumes across connections: collied binds an
/// upload to the session that began it.
pub(crate) async fn upload<C, F>(
    call: C,
    name: AttachmentName,
    data: Vec<u8>,
    progress: Box<dyn UploadProgress>,
    cancel: impl Future<Output = ()>,
) -> Result<String, CoreError>
where
    C: Fn(Request) -> F,
    F: Future<Output = Result<Response, CoreError>>,
{
    let mut started = None;
    let result = tokio::select! {
        r = send(&call, name, &data, &*progress, &mut started) => r,
        () = cancel => Err(CoreError::Cancelled),
    };
    if result.is_err()
        && let Some(upload_id) = started
    {
        let _ = call(Request::AttachmentAbort(AttachmentAbortParams {
            upload_id,
        }))
        .await;
    }
    result
}

async fn send<C, F>(
    call: &C,
    name: AttachmentName,
    data: &[u8],
    progress: &dyn UploadProgress,
    started: &mut Option<UploadId>,
) -> Result<String, CoreError>
where
    C: Fn(Request) -> F,
    F: Future<Output = Result<Response, CoreError>>,
{
    let total = data.len() as u64;
    let sha256 = Sha256Hex::new(hex(&Sha256::digest(data))).expect("64 lowercase hex chars");
    let begin = Request::AttachmentBegin(AttachmentBeginParams {
        op_id: new_op_id(),
        name,
        size: total,
        sha256,
    });
    let upload_id = match call(begin).await? {
        Response::AttachmentStarted { upload_id } => upload_id,
        other => return Err(unexpected(&other).into()),
    };
    *started = Some(upload_id.clone());
    progress.on_progress(0, total);
    let mut offset = 0;
    for chunk in data.chunks(limits::MAX_ATTACHMENT_CHUNK_BYTES) {
        let request = Request::AttachmentChunk(AttachmentChunkParams {
            upload_id: upload_id.clone(),
            offset,
            data: ChunkData::new(STANDARD.encode(chunk)).expect("a chunk encodes within limits"),
        });
        let mut retries = 0;
        loop {
            match call(request.clone()).await {
                Err(CoreError::RateLimited) if retries < RATE_LIMIT_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(RATE_LIMIT_PAUSE).await;
                }
                res => break expect_ok(res.map_err(interrupted)?)?,
            }
        }
        offset += chunk.len() as u64;
        progress.on_progress(offset, total);
    }
    let commit = Request::AttachmentCommit(AttachmentCommitParams {
        op_id: new_op_id(),
        upload_id,
    });
    match call(commit).await.map_err(interrupted)? {
        Response::AttachmentStored { path } => {
            *started = None;
            Ok(path)
        }
        other => Err(unexpected(&other).into()),
    }
}

/// collied answers `not_found` for an upload it dropped: its connection closed, or it
/// sat idle for 60 s.
fn interrupted(e: CoreError) -> CoreError {
    match e {
        CoreError::NotFound => CoreError::Unreachable {
            message: "the upload was interrupted, try again".into(),
        },
        e => e,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use protocol::ErrorCode;

    use super::*;
    use crate::session::{SessionError, lock};

    /// collied's side of the exchange, in memory.
    #[derive(Default)]
    struct Mac {
        expected: Option<(u64, String)>,
        received: Vec<u8>,
        chunks: Vec<(u64, usize)>,
        methods: Vec<&'static str>,
        rate_limited: usize,
        fail_chunk: Option<(usize, ErrorCode)>,
        corrupt: bool,
        stored: Option<Vec<u8>>,
    }

    fn server(code: ErrorCode) -> CoreError {
        SessionError::Server {
            code,
            message: "refused".into(),
        }
        .into()
    }

    impl Mac {
        fn answer(&mut self, request: Request) -> Result<Response, CoreError> {
            self.methods.push(request.method());
            match request {
                Request::AttachmentBegin(p) => {
                    assert!(p.is_valid());
                    self.expected = Some((p.size, p.sha256.as_str().to_owned()));
                    Ok(Response::AttachmentStarted {
                        upload_id: UploadId::new("ab".repeat(16)).unwrap(),
                    })
                }
                Request::AttachmentChunk(p) => {
                    if self.rate_limited > 0 {
                        self.rate_limited -= 1;
                        return Err(server(ErrorCode::RateLimited));
                    }
                    if let Some((n, code)) = self.fail_chunk
                        && self.chunks.len() == n
                    {
                        return Err(server(code));
                    }
                    assert_eq!(p.offset, self.received.len() as u64, "chunks in order");
                    let mut bytes = STANDARD.decode(p.data.as_str()).unwrap();
                    assert!((1..=limits::MAX_ATTACHMENT_CHUNK_BYTES).contains(&bytes.len()));
                    self.chunks.push((p.offset, bytes.len()));
                    if self.corrupt {
                        bytes[0] ^= 1;
                    }
                    self.received.extend(bytes);
                    Ok(Response::Ok)
                }
                Request::AttachmentCommit(_) => {
                    let (size, sha) = self.expected.clone().unwrap();
                    if self.received.len() as u64 != size {
                        return Err(server(ErrorCode::InvalidParams));
                    }
                    if hex(&Sha256::digest(&self.received)) != sha {
                        return Err(server(ErrorCode::ChecksumMismatch));
                    }
                    self.stored = Some(std::mem::take(&mut self.received));
                    Ok(Response::AttachmentStored {
                        path: "/cache/attachments/0123456789abcdef/a.bin".into(),
                    })
                }
                Request::AttachmentAbort(_) => Ok(Response::Ok),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[derive(Default)]
    struct Progress(Mutex<Vec<(u64, u64)>>);

    impl UploadProgress for Arc<Progress> {
        fn on_progress(&self, sent: u64, total: u64) {
            lock(&self.0).push((sent, total));
        }
    }

    async fn run(
        mac: Arc<Mutex<Mac>>,
        data: Vec<u8>,
    ) -> (Result<String, CoreError>, Vec<(u64, u64)>) {
        let progress = Arc::new(Progress::default());
        let call = |request: Request| {
            let mac = mac.clone();
            async move { lock(&mac).answer(request) }
        };
        let name = check("a.bin", &data).unwrap();
        let result = upload(
            call,
            name,
            data,
            Box::new(progress.clone()),
            std::future::pending(),
        )
        .await;
        let seen = lock(&progress.0).clone();
        (result, seen)
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    #[tokio::test]
    async fn chunk_boundaries() {
        let chunk = limits::MAX_ATTACHMENT_CHUNK_BYTES;
        for (len, chunks) in [
            (1, 1),
            (chunk - 1, 1),
            (chunk, 1),
            (chunk + 1, 2),
            (3 * chunk, 3),
            (limits::MAX_ATTACHMENT_BYTES as usize, 640),
        ] {
            let mac = Arc::new(Mutex::new(Mac::default()));
            let data = bytes(len);
            let (result, progress) = run(mac.clone(), data.clone()).await;
            assert_eq!(result.unwrap(), "/cache/attachments/0123456789abcdef/a.bin");
            let mac = lock(&mac);
            assert_eq!(mac.stored.as_deref(), Some(&data[..]), "{len}");
            assert_eq!(mac.chunks.len(), chunks, "{len}");
            assert!(
                mac.chunks
                    .iter()
                    .all(|&(_, n)| n == chunk || n == len % chunk)
            );
            assert_eq!(progress.first(), Some(&(0, len as u64)));
            assert_eq!(progress.last(), Some(&(len as u64, len as u64)));
            assert_eq!(progress.len(), chunks + 1);
            assert!(!mac.methods.contains(&"attachment.abort"));
        }
    }

    #[tokio::test]
    async fn rate_limited_chunks_are_retried() {
        let mac = Arc::new(Mutex::new(Mac {
            rate_limited: 2,
            ..Mac::default()
        }));
        let data = bytes(70_000);
        let (result, _) = run(mac.clone(), data.clone()).await;
        assert!(result.is_ok());
        assert_eq!(lock(&mac).stored.as_deref(), Some(&data[..]));
    }

    #[tokio::test]
    async fn checksum_mismatch_fails_and_aborts() {
        let mac = Arc::new(Mutex::new(Mac {
            corrupt: true,
            ..Mac::default()
        }));
        let (result, _) = run(mac.clone(), bytes(1000)).await;
        assert!(
            matches!(result, Err(CoreError::ChecksumMismatch)),
            "{result:?}"
        );
        let mac = lock(&mac);
        assert!(mac.stored.is_none());
        assert_eq!(mac.methods.last(), Some(&"attachment.abort"));
    }

    #[tokio::test]
    async fn dropped_upload_reads_as_interrupted() {
        let mac = Arc::new(Mutex::new(Mac {
            fail_chunk: Some((1, ErrorCode::NotFound)),
            ..Mac::default()
        }));
        let (result, progress) = run(mac.clone(), bytes(100_000)).await;
        assert!(
            matches!(&result, Err(CoreError::Unreachable { message }) if message.contains("interrupted")),
            "{result:?}"
        );
        assert_eq!(progress.len(), 2, "begin and the first chunk only");
        let mac = lock(&mac);
        assert_eq!(
            mac.methods,
            [
                "attachment.begin",
                "attachment.chunk",
                "attachment.chunk",
                "attachment.abort"
            ]
        );
    }

    #[tokio::test]
    async fn other_chunk_errors_are_final() {
        let mac = Arc::new(Mutex::new(Mac {
            fail_chunk: Some((0, ErrorCode::NotPaired)),
            ..Mac::default()
        }));
        let (result, _) = run(mac.clone(), bytes(10)).await;
        assert!(
            matches!(result, Err(CoreError::Unauthorized { .. })),
            "{result:?}"
        );
        assert_eq!(lock(&mac).methods.len(), 3);
    }

    #[tokio::test]
    async fn cancel_aborts_the_upload() {
        let mac = Arc::new(Mutex::new(Mac::default()));
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        let call = |request: Request| {
            let (mac, tx) = (mac.clone(), tx.clone());
            async move {
                let chunk = matches!(request, Request::AttachmentChunk(_));
                let res = lock(&mac).answer(request);
                if chunk {
                    let _ = tx.send(true);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                res
            }
        };
        let cancel = async move {
            let _ = rx.wait_for(|c| *c).await;
        };
        let data = bytes(100_000);
        let name = check("a.bin", &data).unwrap();
        let progress = Arc::new(Progress::default());
        let result = upload(call, name, data, Box::new(progress), cancel).await;
        assert!(matches!(result, Err(CoreError::Cancelled)), "{result:?}");
        let mac = lock(&mac);
        assert_eq!(mac.methods.last(), Some(&"attachment.abort"));
        assert!(!mac.methods.contains(&"attachment.commit"));
    }

    #[test]
    fn inputs_are_checked_locally() {
        assert!(check(" photo.jpg ", b"x").is_ok());
        for name in ["", "a/b", "..", "x\u{202E}y", &"n".repeat(65)] {
            assert!(
                matches!(check(name, b"x"), Err(CoreError::InvalidInput { field: Some(f), .. }) if f == "name"),
                "{name:?}"
            );
        }
        assert!(matches!(
            check("a", b""),
            Err(CoreError::InvalidInput { .. })
        ));
        let max = limits::MAX_ATTACHMENT_BYTES as usize;
        assert!(check("a", &vec![0; max]).is_ok());
        let err = check("a", &vec![0; max + 1]).unwrap_err();
        assert!(matches!(err, CoreError::TooLarge { .. }));
        assert!(err.to_string().contains("20 MB"), "{err}");
    }
}
