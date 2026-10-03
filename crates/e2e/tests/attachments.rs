#[allow(dead_code)]
mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use collie_core::{CollieCore, CoreError, UploadProgress};
use collied::control::{Client, Reply, Request};
use collied::server::{self, ServerConfig};
use common::*;
use futures_util::{SinkExt, StreamExt};
use protocol::{ErrorCode, PairingInvite, Response, ServerFrame, limits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tailnet::Node;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};

const PROBE: &str = "E2E probe";

#[test]
fn attachments_end_to_end() {
    if !in_child("attachments_end_to_end") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2ea");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let probe = start_node(&root.0, "probe", &net.key, &net.url, &[]);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    let mac_ip = wait_ready(&net.mac, 2)
        .self_node
        .unwrap()
        .tailscale_ips
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    wait_ready(&probe, 1);
    wait_phone(&rt, &core);
    println!(
        "tailnet: control, Mac, phone, probe up in {:?}",
        t0.elapsed()
    );
    let probe = Probe {
        node: probe,
        target: format!("{mac_ip}:{PORT}"),
    };
    rt.block_on(scenario(&root.0, &core, &probe, &net));
    drop(core);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

#[derive(Clone, Default)]
struct Progress(Arc<Mutex<Vec<(u64, u64)>>>);

impl UploadProgress for Progress {
    fn on_progress(&self, sent: u64, total: u64) {
        self.0.lock().unwrap().push((sent, total));
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn upload_dirs(root: &Path) -> usize {
    std::fs::read_dir(root).unwrap().count()
}

async fn scenario(root: &Path, core: &Arc<CollieCore>, probe: &Probe, net: &Net) {
    mock_herdr(&root.join("herdr.sock"));
    let data_dir = root.join("collied");
    let store = root.join("cache/attachments");
    let handle = server::start(
        net.mac.clone(),
        ServerConfig {
            data_dir: data_dir.clone(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "e2e".into(),
            machine_name: "e2e-mac".into(),
            approval_ttl: collied::approvals::TTL,
            attachments_dir: store.clone(),
        },
        root.join("herdr.sock"),
    )
    .await
    .unwrap();
    let control = handle.control_path();
    assert_eq!(mode(&store), 0o700);
    let (machine, _) = pair(&control, core, LABEL).await;
    let m = machine.id.clone();
    connected_flock(core, &m).await;

    println!("the phone uploads 100 KB and gets an absolute path to a private copy");
    let t = Instant::now();
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let progress = Progress::default();
    let path = core
        .upload_attachment(
            m.clone(),
            "Screen Shot 2026.png".into(),
            data.clone(),
            Box::new(progress.clone()),
        )
        .await
        .unwrap();
    let path = std::path::PathBuf::from(path);
    assert!(path.is_absolute());
    assert_eq!(path.parent().unwrap().parent().unwrap(), store);
    assert_eq!(path.file_name().unwrap(), "Screen-Shot-2026.png");
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    let seen = progress.0.lock().unwrap().clone();
    assert_eq!(seen.first(), Some(&(0, 100_000)));
    assert_eq!(seen.last(), Some(&(100_000, 100_000)));
    assert_eq!(seen.len(), 5, "begin and four chunks");
    let audit = data_dir.join("audit.log");
    let commits: Vec<Value> = audit_lines(&audit)
        .into_iter()
        .filter(|e| e["method"] == "attachment.commit")
        .collect();
    assert_eq!(commits.len(), 1, "{commits:#?}");
    assert_eq!(commits[0]["target"], "Screen-Shot-2026.png");
    assert_eq!(
        commits[0]["result"],
        format!("ok size=100000 sha256={}", &sha256(&data)[..12])
    );
    let log = std::fs::read_to_string(&audit).unwrap();
    assert!(!log.contains(&STANDARD.encode(&data[..48])));
    assert!(
        !audit_lines(&audit)
            .iter()
            .any(|e| e["method"] == "attachment.chunk")
    );
    println!("  uploaded in {:?}", t.elapsed());

    println!("an oversized file is refused before anything is sent");
    let big = vec![0u8; limits::MAX_ATTACHMENT_BYTES as usize + 1];
    let err = core
        .upload_attachment(m.clone(), "big.bin".into(), big, Box::new(progress.clone()))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::TooLarge { .. }), "{err:?}");

    probe.pair(&control).await;
    let mut a = probe.session().await;
    let mut b = probe.session().await;

    println!("a second session cannot chunk, commit or abort another session's upload");
    let upload_id = begin(&mut a, 1, "notes.txt", 3, &sha256(b"abc"))
        .await
        .unwrap();
    assert_eq!(
        chunk(&mut b, &upload_id, 0, b"abc").await,
        Err(ErrorCode::NotFound)
    );
    assert_eq!(
        commit(&mut b, 2, &upload_id).await,
        Err(ErrorCode::NotFound)
    );
    let abort = call(&mut b, "attachment.abort", json!({"upload_id": upload_id})).await;
    assert_eq!(abort, Ok(Response::Ok), "best effort, and a no-op here");
    chunk(&mut a, &upload_id, 0, b"abc").await.unwrap();
    let Ok(Response::AttachmentStored { path }) = commit(&mut a, 3, &upload_id).await else {
        panic!("commit failed");
    };
    assert_eq!(std::fs::read(&path).unwrap(), b"abc");
    let stored = upload_dirs(&store);
    assert_eq!(stored, 2);

    println!("a checksum mismatch leaves no file");
    let upload_id = begin(&mut a, 4, "x.txt", 3, &sha256(b"abd")).await.unwrap();
    assert_eq!(upload_dirs(&store), stored + 1);
    chunk(&mut a, &upload_id, 0, b"abc").await.unwrap();
    assert_eq!(
        commit(&mut a, 5, &upload_id).await,
        Err(ErrorCode::ChecksumMismatch)
    );
    assert_eq!(upload_dirs(&store), stored);
    assert_eq!(
        chunk(&mut a, &upload_id, 3, b"d").await,
        Err(ErrorCode::NotFound)
    );

    println!("an oversized begin is refused");
    let max = limits::MAX_ATTACHMENT_BYTES;
    assert_eq!(
        begin(&mut a, 6, "big.bin", max + 1, &sha256(b"")).await,
        Err(ErrorCode::InvalidParams)
    );
    assert_eq!(upload_dirs(&store), stored);

    println!("closing a session drops its uploads");
    let mut c = probe.session().await;
    begin(&mut c, 7, "y.txt", 3, &sha256(b"abc")).await.unwrap();
    assert_eq!(upload_dirs(&store), stored + 1);
    drop(c);
    let deadline = Instant::now() + Duration::from_secs(10);
    while upload_dirs(&store) != stored {
        assert!(Instant::now() < deadline, "partial upload left after close");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop((a, b));
    handle.shutdown().await;
}

fn op_id(n: u32) -> String {
    format!("{n:0>22}")
}

async fn begin(
    ws: &mut Ws,
    n: u32,
    name: &str,
    size: u64,
    sha256: &str,
) -> Result<String, ErrorCode> {
    let params = json!({"op_id": op_id(n), "name": name, "size": size, "sha256": sha256});
    match call(ws, "attachment.begin", params).await? {
        Response::AttachmentStarted { upload_id } => Ok(upload_id.as_str().to_owned()),
        other => panic!("attachment.begin: {other:?}"),
    }
}

async fn chunk(ws: &mut Ws, upload_id: &str, offset: u64, data: &[u8]) -> Result<(), ErrorCode> {
    let params = json!({"upload_id": upload_id, "offset": offset, "data": STANDARD.encode(data)});
    call(ws, "attachment.chunk", params).await.map(|_| ())
}

async fn commit(ws: &mut Ws, n: u32, upload_id: &str) -> Result<Response, ErrorCode> {
    let params = json!({"op_id": op_id(n), "upload_id": upload_id});
    call(ws, "attachment.commit", params).await
}

type Ws = WebSocketStream<UnixStream>;

struct Probe {
    node: Node,
    target: String,
}

impl Probe {
    async fn session(&self) -> Ws {
        let stream = {
            let (node, target) = (self.node.clone(), self.target.clone());
            // netstack occasionally stalls one SYN for ~63 s; bounded retries keep the test fast.
            tokio::task::spawn_blocking(move || {
                (0..6)
                    .find_map(|_| {
                        node.dial_timeout("tcp", &target, Duration::from_secs(10))
                            .ok()
                    })
                    .expect("probe could not dial collied")
            })
            .await
            .unwrap()
        };
        stream.set_nonblocking(true).unwrap();
        let mut req = format!("ws://{}{}", self.target, protocol::WS_PATH)
            .into_client_request()
            .unwrap();
        req.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(protocol::WS_SUBPROTOCOL),
        );
        let (mut ws, _) =
            tokio_tungstenite::client_async(req, UnixStream::from_std(stream).unwrap())
                .await
                .unwrap();
        let hello = json!({"protocol_version": protocol::PROTOCOL_VERSION, "app_version": "e2e"});
        call(&mut ws, "hello", hello).await.unwrap();
        ws
    }

    async fn pair(&self, control: &Path) {
        let mut cli = Client::connect(control).await.unwrap();
        let Reply::Invite { uri, .. } = cli.call(&Request::Pair).await.unwrap() else {
            panic!("no invite");
        };
        let code = PairingInvite::parse(&uri).unwrap().code;
        let mut ws = self.session().await;
        let params = json!({"pairing_code": code.as_str(), "device_label": PROBE});
        let (paired, ()) = tokio::join!(call(&mut ws, "pair.complete", params), async {
            let Reply::Confirm(candidate) = cli.recv().await.unwrap() else {
                panic!("no confirmation request");
            };
            assert_eq!(candidate.device_label, PROBE);
            cli.send(&Request::Confirm { accept: true }).await.unwrap();
            assert!(matches!(
                cli.recv().await.unwrap(),
                Reply::PairDone { paired: true, .. }
            ));
        });
        assert!(matches!(paired, Ok(Response::Paired { .. })), "{paired:?}");
    }
}

async fn call(ws: &mut Ws, method: &str, params: Value) -> Result<Response, ErrorCode> {
    let frame = json!({"id": 1, "method": method, "params": params});
    ws.send(Message::text(frame.to_string())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("no frame within 15 s")
            .expect("connection ended")
            .expect("websocket error");
        let Message::Text(text) = msg else { continue };
        match serde_json::from_str::<ServerFrame>(text.as_str()).unwrap() {
            ServerFrame::Result { result, .. } => return Ok(result),
            ServerFrame::Error { error, .. } => return Err(error.code),
            ServerFrame::Event { .. } => {}
        }
    }
}

// One request per connection, like herdr. Fixtures are sanitized live samples.
fn mock_herdr(path: &Path) {
    let listener = UnixListener::bind(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let mut line = String::new();
                if tokio::io::BufReader::new(r)
                    .read_line(&mut line)
                    .await
                    .is_err()
                {
                    return;
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                let fixture = match req["method"].as_str().unwrap_or_default() {
                    "ping" => Some(include_str!("fixtures/ping.json")),
                    "session.snapshot" => Some(include_str!("fixtures/session.snapshot.json")),
                    "agent.list" => Some(include_str!("fixtures/agent.list.json")),
                    "workspace.list" => Some(include_str!("fixtures/workspace.list.json")),
                    _ => None,
                };
                let mut resp = match fixture {
                    Some(text) => serde_json::from_str(text).unwrap(),
                    None => {
                        json!({"error": {"code": "unknown_method", "message": "not mocked"}})
                    }
                };
                resp["id"] = req["id"].clone();
                let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
            });
        }
    });
}
