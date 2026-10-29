use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::{StreamExt, TryStreamExt, stream};
use protocol::{
    DownloadId, FileChunkParams, FileCloseParams, FileOpenParams, Request, Response, hex, limits,
};
use sha2::{Digest, Sha256};

use crate::session::unexpected;
use crate::{CoreError, FileKind, OpenedFile};

const IN_FLIGHT: usize = 4;
const RATE_LIMIT_RETRIES: usize = 8;
const RATE_LIMIT_PAUSE: Duration = Duration::from_millis(250);

/// open, chunks (a few in flight), close. The file is checked against collied's size and
/// checksum before it is returned; nothing is written to disk. Nothing resumes across
/// connections: collied binds an open file to the session that opened it.
pub(crate) async fn download<C, F>(
    call: C,
    params: FileOpenParams,
    cancel: impl Future<Output = ()>,
) -> Result<OpenedFile, CoreError>
where
    C: Fn(Request) -> F,
    F: Future<Output = Result<Response, CoreError>>,
{
    let mut opened = None;
    let result = tokio::select! {
        r = receive(&call, params, &mut opened) => r,
        () = cancel => Err(CoreError::Cancelled),
    };
    if let Some(download_id) = opened {
        let _ = call(Request::FileClose(FileCloseParams { download_id })).await;
    }
    result
}

async fn receive<C, F>(
    call: &C,
    params: FileOpenParams,
    opened: &mut Option<DownloadId>,
) -> Result<OpenedFile, CoreError>
where
    C: Fn(Request) -> F,
    F: Future<Output = Result<Response, CoreError>>,
{
    let (download_id, name, size, kind, sha256) = match call(Request::FileOpen(params)).await? {
        Response::FileOpened {
            download_id,
            name,
            size,
            kind,
            sha256,
        } => (download_id, name, size, kind, sha256),
        other => return Err(unexpected(&other).into()),
    };
    *opened = Some(download_id.clone());
    if size > limits::MAX_FILE_BYTES {
        return Err(CoreError::TooLarge {
            message: format!("{name} is too large to open"),
        });
    }
    let chunk = limits::MAX_ATTACHMENT_CHUNK_BYTES as u64;
    let chunks: Vec<Vec<u8>> = stream::iter((0..size).step_by(chunk as usize))
        .map(|offset| {
            let request = Request::FileChunk(FileChunkParams {
                download_id: download_id.clone(),
                offset,
            });
            async move {
                let mut retries = 0;
                let data = loop {
                    match call(request.clone()).await {
                        Err(CoreError::RateLimited) if retries < RATE_LIMIT_RETRIES => {
                            retries += 1;
                            tokio::time::sleep(RATE_LIMIT_PAUSE).await;
                        }
                        Ok(Response::FileChunk { data }) => break data,
                        Ok(other) => return Err(unexpected(&other).into()),
                        Err(e) => return Err(interrupted(e)),
                    }
                };
                let bytes = STANDARD
                    .decode(data.as_str())
                    .map_err(|_| CoreError::ChecksumMismatch)?;
                if bytes.len() as u64 != chunk.min(size - offset) {
                    return Err(CoreError::ChecksumMismatch);
                }
                Ok(bytes)
            }
        })
        .buffered(IN_FLIGHT)
        .try_collect()
        .await?;
    let data = chunks.concat();
    if hex(&Sha256::digest(&data)) != sha256.as_str() {
        return Err(CoreError::ChecksumMismatch);
    }
    let name = match name.rsplit('/').next() {
        Some(n) if !n.is_empty() && !n.starts_with('.') && !n.contains('\0') => n.to_owned(),
        _ => "file".to_owned(),
    };
    Ok(OpenedFile {
        name,
        size,
        kind: match kind {
            protocol::FileKind::Image => FileKind::Image,
            protocol::FileKind::Pdf => FileKind::Pdf,
            protocol::FileKind::Text => FileKind::Text,
            protocol::FileKind::Other => FileKind::Other,
        },
        data,
    })
}

/// collied answers `not_found` for a file it closed: its connection closed, or it sat
/// idle for 60 s.
fn interrupted(e: CoreError) -> CoreError {
    match e {
        CoreError::NotFound => CoreError::Unreachable {
            message: "the transfer was interrupted, try again".into(),
        },
        e => e,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use protocol::{ChunkData, ErrorCode, Sha256Hex, TerminalId};

    use super::*;
    use crate::session::{SessionError, lock};

    #[derive(Default)]
    struct Mac {
        file: Vec<u8>,
        methods: Vec<&'static str>,
        offsets: Vec<u64>,
        rate_limited: usize,
        fail_chunk: Option<(usize, ErrorCode)>,
        corrupt: bool,
        wrong_sha: bool,
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
                Request::FileOpen(p) => {
                    assert!(p.is_valid());
                    let digest = if self.wrong_sha {
                        "0".repeat(64)
                    } else {
                        hex(&Sha256::digest(&self.file))
                    };
                    Ok(Response::FileOpened {
                        download_id: DownloadId::new("ab".repeat(16)).unwrap(),
                        name: "shot.png".into(),
                        size: self.file.len() as u64,
                        kind: protocol::FileKind::Image,
                        sha256: Sha256Hex::new(digest).unwrap(),
                    })
                }
                Request::FileChunk(p) => {
                    if self.rate_limited > 0 {
                        self.rate_limited -= 1;
                        return Err(server(ErrorCode::RateLimited));
                    }
                    if let Some((n, code)) = self.fail_chunk
                        && self.offsets.len() == n
                    {
                        return Err(server(code));
                    }
                    self.offsets.push(p.offset);
                    let start = p.offset as usize;
                    let end = (start + limits::MAX_ATTACHMENT_CHUNK_BYTES).min(self.file.len());
                    let mut bytes = self.file[start..end].to_vec();
                    if self.corrupt {
                        bytes[0] ^= 1;
                    }
                    Ok(Response::FileChunk {
                        data: ChunkData::new(STANDARD.encode(bytes)).unwrap(),
                    })
                }
                Request::FileClose(_) => Ok(Response::Ok),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    fn params() -> FileOpenParams {
        FileOpenParams {
            terminal_id: TerminalId::new("term_1").unwrap(),
            path: "shot.png".into(),
        }
    }

    async fn run(mac: Arc<Mutex<Mac>>) -> Result<OpenedFile, CoreError> {
        let call = |request: Request| {
            let mac = mac.clone();
            async move { lock(&mac).answer(request) }
        };
        download(call, params(), std::future::pending()).await
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    #[tokio::test]
    async fn chunk_boundaries_and_checksum() {
        let chunk = limits::MAX_ATTACHMENT_CHUNK_BYTES;
        for (len, chunks) in [
            (0, 0),
            (1, 1),
            (chunk, 1),
            (chunk + 1, 2),
            (5 * chunk + 7, 6),
        ] {
            let mac = Arc::new(Mutex::new(Mac {
                file: bytes(len),
                ..Mac::default()
            }));
            let opened = run(mac.clone()).await.unwrap();
            assert_eq!(opened.data, bytes(len), "{len}");
            assert_eq!(opened.size, len as u64);
            assert_eq!(opened.name, "shot.png");
            assert_eq!(opened.kind, FileKind::Image);
            let mac = lock(&mac);
            let mut offsets = mac.offsets.clone();
            offsets.sort();
            assert_eq!(
                offsets,
                (0..chunks as u64)
                    .map(|i| i * chunk as u64)
                    .collect::<Vec<_>>()
            );
            assert_eq!(mac.methods.first(), Some(&"file.open"));
            assert_eq!(mac.methods.last(), Some(&"file.close"));
        }
    }

    #[tokio::test]
    async fn rate_limited_chunks_are_retried() {
        let mac = Arc::new(Mutex::new(Mac {
            file: bytes(100_000),
            rate_limited: 3,
            ..Mac::default()
        }));
        assert_eq!(run(mac).await.unwrap().data, bytes(100_000));
    }

    #[tokio::test]
    async fn corrupt_or_changed_files_fail_and_close() {
        for (corrupt, wrong_sha) in [(true, false), (false, true)] {
            let mac = Arc::new(Mutex::new(Mac {
                file: bytes(1000),
                corrupt,
                wrong_sha,
                ..Mac::default()
            }));
            let result = run(mac.clone()).await;
            assert!(
                matches!(result, Err(CoreError::ChecksumMismatch)),
                "{result:?}"
            );
            assert_eq!(lock(&mac).methods.last(), Some(&"file.close"));
        }
    }

    #[tokio::test]
    async fn a_closed_transfer_reads_as_interrupted_and_can_be_retried() {
        let mac = Arc::new(Mutex::new(Mac {
            file: bytes(100_000),
            fail_chunk: Some((1, ErrorCode::NotFound)),
            ..Mac::default()
        }));
        let result = run(mac.clone()).await;
        assert!(
            matches!(&result, Err(CoreError::Unreachable { message }) if message.contains("interrupted")),
            "{result:?}"
        );
        assert_eq!(lock(&mac).methods.last(), Some(&"file.close"));
        lock(&mac).fail_chunk = None;
        assert_eq!(run(mac).await.unwrap().data, bytes(100_000));
    }

    #[tokio::test]
    async fn refusals_are_final_and_nothing_is_closed() {
        let call = |_: Request| async { Err(server(ErrorCode::TooLarge)) };
        let result = download(call, params(), std::future::pending()).await;
        assert!(
            matches!(result, Err(CoreError::TooLarge { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn cancel_closes_the_file() {
        let mac = Arc::new(Mutex::new(Mac {
            file: bytes(100_000),
            ..Mac::default()
        }));
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        let call = |request: Request| {
            let (mac, tx) = (mac.clone(), tx.clone());
            async move {
                let chunk = matches!(request, Request::FileChunk(_));
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
        let result = download(call, params(), cancel).await;
        assert!(matches!(result, Err(CoreError::Cancelled)), "{result:?}");
        assert_eq!(lock(&mac).methods.last(), Some(&"file.close"));
    }
}
