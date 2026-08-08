//! relay-v1: strict request/response frames over the magic-wormhole record
//! pipe. One in-flight request, no ids, no pipelining. The tree is immutable
//! per session. EOF (close or drop) ends the session.
//!
//! Two sides:
//! - `server_loop`: the `serve` side. Reads requests off the pipe, answers
//!   list/get from the (immutable) index.
//! - `reader_loop` + `Client`: the `open` side. The wormhole lives on its own
//!   thread; the HTTP thread talks to it through std mpsc channels. Strict
//!   alternation is guaranteed by the Client mutex (one request in flight).

use std::future::Future;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;

use magic_wormhole::{AppConfig, AppID, Wormhole, WormholeError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::index::{content_type, Doc, Index, MAX_DOC_SIZE};
use crate::{APP_ID, APP_VERSION, PAIRING_TIMEOUT};

pub const PROTOCOL_VERSION: u64 = 1;

// ---------- frames ----------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Hello {
    #[serde(rename = "type")]
    pub kind: String,
    pub v: u64,
    pub role: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tree {
    #[serde(rename = "type")]
    pub kind: String,
    pub docs: Vec<Doc>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Data {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------- pipe ----------

/// The record channel abstraction. Implemented by the real wormhole and by
/// in-memory channels in tests. All calls block the calling thread.
pub trait Pipe {
    fn send_json(&mut self, frame: &Value) -> Result<(), String>;
    fn recv_json(&mut self) -> Result<Value, String>;
    fn send_bytes(&mut self, bytes: Vec<u8>) -> Result<(), String>;
    fn recv_bytes(&mut self) -> Result<Vec<u8>, String>;
}

pub struct WormholePipe(pub Wormhole);

impl Pipe for WormholePipe {
    fn send_json(&mut self, frame: &Value) -> Result<(), String> {
        async_io::block_on(self.0.send_json(frame)).map_err(|e| e.to_string())
    }
    fn recv_json(&mut self) -> Result<Value, String> {
        async_io::block_on(self.0.receive_json())
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    fn send_bytes(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        async_io::block_on(self.0.send(bytes)).map_err(|e| e.to_string())
    }
    fn recv_bytes(&mut self) -> Result<Vec<u8>, String> {
        async_io::block_on(self.0.receive()).map_err(|e| e.to_string())
    }
}

// ---------- pairing helpers ----------

pub fn config(rendezvous: &str) -> AppConfig<&'static str> {
    AppConfig {
        id: AppID::new(APP_ID),
        rendezvous_url: rendezvous.to_string().into(),
        app_version: APP_VERSION,
    }
}

pub fn map_pairing_error(e: &WormholeError) -> String {
    match e {
        WormholeError::CodeInvalid(_) => "invalid pairing code".to_string(),
        WormholeError::UnclaimedNameplate(_) => "wrong or expired pairing code".to_string(),
        WormholeError::ServerError(_) => "rendezvous server unreachable".to_string(),
        WormholeError::PakeFailed | WormholeError::Crypto => "pairing failed".to_string(),
        other => format!("pairing failed: {other}"),
    }
}

/// Run a pairing future with a 10-minute deadline. The future must be `Send + 'static`
/// because it is boxed (the wormhole client may hold borrowed state).
pub fn with_timeout<T>(
    fut: impl Future<Output = Result<T, WormholeError>> + Send + 'static,
) -> impl Future<Output = Result<T, String>> + Send {
    use futures_lite::future::FutureExt;
    futures_lite::future::or(
        async move { fut.await.map_err(|e| map_pairing_error(&e)) }.boxed(),
        async {
            async_io::Timer::after(PAIRING_TIMEOUT).await;
            Err::<T, String>("pairing timed out after 10 minutes".to_string())
        }
        .boxed(),
    )
}

// ---------- shared fetch ----------

pub struct FetchedDoc {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub error: Option<String>,
    pub body: Option<Vec<u8>>,
}

pub fn fetch_doc(index: &Index, id: u64) -> FetchedDoc {
    let Some(&slot) = index.by_id.get(&id) else {
        return FetchedDoc {
            status: 404,
            content_type: None,
            error: Some("no such doc".to_string()),
            body: None,
        };
    };
    let doc = &index.docs[slot];
    if doc.size > MAX_DOC_SIZE {
        return FetchedDoc {
            status: 413,
            content_type: None,
            error: Some("doc too large to transfer".to_string()),
            body: None,
        };
    }
    match std::fs::read(&doc.abs) {
        Ok(body) => FetchedDoc {
            status: 200,
            content_type: Some(content_type(&doc.kind)),
            error: None,
            body: Some(body),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FetchedDoc {
            status: 404,
            content_type: None,
            error: Some("doc vanished between index and fetch".to_string()),
            body: None,
        },
        Err(_) => FetchedDoc {
            status: 500,
            content_type: None,
            error: Some("failed to read doc".to_string()),
            body: None,
        },
    }
}

// ---------- server side ----------

/// Serve one paired session: hello exchange, then answer list/get until EOF.
pub fn server_loop<P: Pipe>(mut pipe: P, index: &Index) -> Result<(), String> {
    pipe.send_json(&json!({"type": "hello", "v": PROTOCOL_VERSION, "role": "server"}))?;
    let hello: Hello = serde_json::from_value(pipe.recv_json()?)
        .map_err(|e| format!("bad reader hello: {e}"))?;
    if hello.v != PROTOCOL_VERSION {
        let msg = format!(
            "server speaks protocol {PROTOCOL_VERSION}, you speak {}",
            hello.v
        );
        let _ = pipe.send_json(&json!({"type": "error", "message": msg}));
        return Err(msg);
    }
    loop {
        let request: Value = pipe.recv_json()?;
        match request.get("type").and_then(|t| t.as_str()) {
            Some("list") => {
                pipe.send_json(&json!({"type": "tree", "docs": index.docs}))?;
            }
            Some("get") => {
                let id = request
                    .get("id")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(u64::MAX);
                let doc = fetch_doc(index, id);
                pipe.send_json(&json!({
                    "type": "data",
                    "status": doc.status,
                    "content_type": doc.content_type,
                    "size": doc.body.as_ref().map(|b| b.len() as u64),
                    "error": doc.error,
                }))?;
                // Strict framing: every get is answered by exactly one data
                // frame followed by exactly one body record (empty on error).
                pipe.send_bytes(doc.body.unwrap_or_default())?;
            }
            Some("bye") => break,
            _ => {
                pipe.send_json(&json!({"type": "error", "message": "unknown request"}))?;
            }
        }
    }
    Ok(())
}

// ---------- client side ----------

#[derive(Debug)]
pub enum GetResult {
    Ok {
        content_type: String,
        /// Doc size from the data frame (the body length is authoritative for
        /// rendering; this stays as part of the relay-v1 contract).
        #[allow(dead_code)]
        size: u64,
        body: Vec<u8>,
    },
    Err {
        status: u16,
        message: String,
    },
}

#[derive(Debug)]
pub enum ClientRequest {
    List,
    Get(u64),
}

#[derive(Debug)]
pub enum ClientResponse {
    /// First message on the channel: handshake outcome.
    Ready(Result<(), String>),
    Tree(Vec<Doc>),
    Doc(GetResult),
    Failed(String),
}

/// Runs on the wormhole thread. Pairs, performs the reader hello handshake,
/// reports the outcome, then relays typed requests to the pipe.
pub fn reader_loop<P: Pipe>(
    mut pipe: P,
    req_rx: Receiver<ClientRequest>,
    resp_tx: Sender<ClientResponse>,
) -> Result<(), String> {
    let handshake = (|| -> Result<(), String> {
        let hello: Hello = serde_json::from_value(pipe.recv_json()?)
            .map_err(|e| format!("bad server hello: {e}"))?;
        if hello.v != PROTOCOL_VERSION {
            return Err(format!(
                "reader speaks protocol {PROTOCOL_VERSION}, you speak {}",
                hello.v
            ));
        }
        pipe.send_json(&json!({"type": "hello", "v": PROTOCOL_VERSION, "role": "reader"}))
    })();
    match handshake {
        Ok(()) => {
            let _ = resp_tx.send(ClientResponse::Ready(Ok(())));
        }
        Err(e) => {
            let _ = resp_tx.send(ClientResponse::Ready(Err(e.clone())));
            return Err(e);
        }
    }
    while let Ok(request) = req_rx.recv() {
        let response = match request {
            ClientRequest::List => {
                pipe.send_json(&json!({"type": "list"}))?;
                match serde_json::from_value::<Tree>(pipe.recv_json()?) {
                    Ok(tree) => ClientResponse::Tree(tree.docs),
                    Err(e) => ClientResponse::Failed(format!("bad tree frame: {e}")),
                }
            }
            ClientRequest::Get(id) => {
                pipe.send_json(&json!({"type": "get", "id": id}))?;
                match serde_json::from_value::<Data>(pipe.recv_json()?) {
                    Ok(data) => {
                        // Strict framing: exactly one body record follows
                        // every data frame (empty for error statuses).
                        let body = match pipe.recv_bytes() {
                            Ok(body) => body,
                            Err(e) => {
                                return Err(format!("pipe closed mid-body: {e}"));
                            }
                        };
                        if data.status == 200 {
                            ClientResponse::Doc(GetResult::Ok {
                                content_type: data
                                    .content_type
                                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                                size: data.size.unwrap_or(body.len() as u64),
                                body,
                            })
                        } else {
                            ClientResponse::Doc(GetResult::Err {
                                status: data.status,
                                message: data
                                    .error
                                    .unwrap_or_else(|| "doc unavailable".to_string()),
                            })
                        }
                    }
                    Err(e) => ClientResponse::Failed(format!("bad data frame: {e}")),
                }
            }
        };
        if resp_tx.send(response).is_err() {
            break; // HTTP side gone
        }
    }
    Ok(())
}

/// HTTP-side handle: one request in flight, serialized by a mutex.
pub struct Client {
    req: Mutex<Sender<ClientRequest>>,
    resp: Mutex<Receiver<ClientResponse>>,
}

impl Client {
    pub fn new(req: Sender<ClientRequest>, resp: Receiver<ClientResponse>) -> Self {
        Self {
            req: Mutex::new(req),
            resp: Mutex::new(resp),
        }
    }

    pub fn list(&self) -> Result<Vec<Doc>, String> {
        let req = self.req.lock().unwrap();
        let resp = self.resp.lock().unwrap();
        req.send(ClientRequest::List).map_err(|e| e.to_string())?;
        match resp.recv().map_err(|e| e.to_string())? {
            ClientResponse::Tree(docs) => Ok(docs),
            ClientResponse::Failed(e) => Err(e),
            _ => Err("protocol desync".to_string()),
        }
    }

    pub fn get(&self, id: u64) -> Result<GetResult, String> {
        let req = self.req.lock().unwrap();
        let resp = self.resp.lock().unwrap();
        req.send(ClientRequest::Get(id)).map_err(|e| e.to_string())?;
        match resp.recv().map_err(|e| e.to_string())? {
            ClientResponse::Doc(result) => Ok(result),
            ClientResponse::Failed(e) => Err(e),
            _ => Err("protocol desync".to_string()),
        }
    }
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::index_dir;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::mpsc::{Receiver as ChanRx, Sender as ChanTx};

    struct ChanPipe {
        tx: ChanTx<Vec<u8>>,
        rx: ChanRx<Vec<u8>>,
    }

    impl Pipe for ChanPipe {
        fn send_json(&mut self, frame: &Value) -> Result<(), String> {
            self.send_bytes(frame.to_string().into_bytes())
        }
        fn recv_json(&mut self) -> Result<Value, String> {
            let bytes = self.recv_bytes()?;
            serde_json::from_slice(&bytes).map_err(|e| format!("bad json: {e}"))
        }
        fn send_bytes(&mut self, bytes: Vec<u8>) -> Result<(), String> {
            self.tx.send(bytes).map_err(|e| e.to_string())
        }
        fn recv_bytes(&mut self) -> Result<Vec<u8>, String> {
            self.rx.recv().map_err(|e| e.to_string())
        }
    }

    fn pipe_pair() -> (ChanPipe, ChanPipe) {
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        (
            ChanPipe { tx: a_tx, rx: b_rx },
            ChanPipe { tx: b_tx, rx: a_rx },
        )
    }

    fn temp_corpus() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("docscrying-protocol-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join("README.md"), "# hi\n\nbody text\n").unwrap();
        fs::write(dir.join("docs/guide.rst"), "Guide\n=====\n\ncontent\n").unwrap();
        dir
    }

    /// Spawn a full server session and a reader session over in-memory pipes.
    /// Returns the Client (HTTP side) plus the server thread result handle.
    fn run_session(index: Index) -> (Client, std::thread::JoinHandle<Result<(), String>>) {
        let (server_pipe, client_pipe) = pipe_pair();
        let server = std::thread::spawn(move || server_loop(server_pipe, &index));
        let (req_tx, req_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();
        let _reader = std::thread::spawn(move || reader_loop(client_pipe, req_rx, resp_tx));
        let client = Client::new(req_tx, resp_rx);
        // consume the handshake outcome
        match client.resp.lock().unwrap().recv().unwrap() {
            ClientResponse::Ready(Ok(())) => {}
            ClientResponse::Ready(Err(e)) => panic!("handshake failed: {e}"),
            other => panic!("expected Ready, got {other:?}"),
        }
        (client, server)
    }

    #[test]
    fn hello_mismatch_reports_both_versions() {
        let (server_pipe, mut client_pipe) = pipe_pair();
        let dir = temp_corpus();
        let index = index_dir(&dir).unwrap();
        let server = std::thread::spawn(move || server_loop(server_pipe, &index));

        let hello: Hello = serde_json::from_value(client_pipe.recv_json().unwrap()).unwrap();
        assert_eq!(hello.kind, "hello");
        assert_eq!(hello.role, "server");
        assert_eq!(hello.v, PROTOCOL_VERSION);

        client_pipe
            .send_json(&json!({"type": "hello", "v": 2, "role": "reader"}))
            .unwrap();
        let error: Value = client_pipe.recv_json().unwrap();
        assert_eq!(error["type"], "error");
        let message = error["message"].as_str().unwrap();
        assert!(
            message.contains("server speaks protocol 1, you speak 2"),
            "got: {message}"
        );

        let err = server.join().unwrap().expect_err("server must reject mismatch");
        assert!(err.contains("server speaks protocol 1, you speak 2"), "got: {err}");
    }

    #[test]
    fn reader_handshake_rejects_mismatched_server() {
        let (server_pipe, client_pipe) = pipe_pair();
        let (_req_tx, req_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut pipe = server_pipe;
            // act like a protocol-2 server
            pipe.send_json(&json!({"type": "hello", "v": 2, "role": "server"}))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
        let reader = std::thread::spawn(move || reader_loop(client_pipe, req_rx, resp_tx));

        match resp_rx.recv().unwrap() {
            ClientResponse::Ready(Err(e)) => {
                assert!(
                    e.contains("reader speaks protocol 1, you speak 2"),
                    "got: {e}"
                );
            }
            other => panic!("expected Ready(Err), got {other:?}"),
        }
        let err = reader.join().unwrap().expect_err("reader must reject mismatch");
        assert!(err.contains("reader speaks protocol 1, you speak 2"), "got: {err}");
        server.join().unwrap();
    }

    #[test]
    fn list_tree_roundtrip() {
        let dir = temp_corpus();
        let index = index_dir(&dir).unwrap();
        let expected: Vec<Doc> = index.docs.clone();
        let (client, server) = run_session(index);

        let docs = client.list().unwrap();
        assert_eq!(docs.len(), expected.len());
        for (got, want) in docs.iter().zip(expected.iter()) {
            assert_eq!(got.id, want.id);
            assert_eq!(got.path, want.path);
            assert_eq!(got.kind, want.kind);
            assert_eq!(got.size, want.size);
            assert_eq!(got.mtime, want.mtime);
        }
        drop(client);
        let _ = server.join().unwrap();
    }

    #[test]
    fn get_ok_returns_body_with_content_type() {
        let dir = temp_corpus();
        let index = index_dir(&dir).unwrap();
        let readme_id = index.docs.iter().find(|d| d.path == "README.md").unwrap().id;
        let (client, server) = run_session(index);

        match client.get(readme_id).unwrap() {
            GetResult::Ok {
                content_type,
                size,
                body,
            } => {
                assert_eq!(content_type, "text/markdown");
                assert_eq!(size, b"# hi\n\nbody text\n".len() as u64);
                assert_eq!(body, b"# hi\n\nbody text\n");
            }
            _ => panic!("expected a 200 body"),
        }
        drop(client);
        let _ = server.join().unwrap();
    }

    #[test]
    fn get_missing_doc_is_404() {
        let dir = temp_corpus();
        let index = index_dir(&dir).unwrap();
        let readme_id = index.docs.iter().find(|d| d.path == "README.md").unwrap().id;
        fs::remove_file(dir.join("README.md")).unwrap();
        let (client, server) = run_session(index);

        match client.get(readme_id).unwrap() {
            GetResult::Err { status, message } => {
                assert_eq!(status, 404);
                assert!(message.contains("vanished"), "got: {message}");
            }
            _ => panic!("expected 404"),
        }
        drop(client);
        let _ = server.join().unwrap();
    }

    #[test]
    fn get_unknown_id_is_404() {
        let dir = temp_corpus();
        let index = index_dir(&dir).unwrap();
        let (client, server) = run_session(index);

        match client.get(999).unwrap() {
            GetResult::Err { status, .. } => assert_eq!(status, 404),
            _ => panic!("expected 404"),
        }
        drop(client);
        let _ = server.join().unwrap();
    }

    #[test]
    fn get_oversized_doc_is_413() {
        let dir = temp_corpus();
        let index = Index {
            docs: vec![Doc {
                id: 1,
                path: "README.md".to_string(),
                kind: "md".to_string(),
                size: MAX_DOC_SIZE + 1,
                mtime: 0,
                abs: dir.join("README.md"),
            }],
            by_id: HashMap::from([(1, 0)]),
        };
        let (client, server) = run_session(index);

        match client.get(1).unwrap() {
            GetResult::Err { status, message } => {
                assert_eq!(status, 413);
                assert!(message.contains("too large"), "got: {message}");
            }
            _ => panic!("expected 413"),
        }
        drop(client);
        let _ = server.join().unwrap();
    }

    #[test]
    fn pairing_error_mapping() {
        let bad_code = "x".parse::<magic_wormhole::Code>().unwrap_err();
        assert_eq!(
            map_pairing_error(&WormholeError::CodeInvalid(bad_code)),
            "invalid pairing code"
        );
        assert_eq!(
            map_pairing_error(&WormholeError::UnclaimedNameplate(
                "1".parse().unwrap()
            )),
            "wrong or expired pairing code"
        );
        assert_eq!(
            map_pairing_error(&WormholeError::ServerError(
                magic_wormhole::rendezvous::RendezvousError::Protocol("x".into())
            )),
            "rendezvous server unreachable"
        );
        assert_eq!(
            map_pairing_error(&WormholeError::PakeFailed),
            "pairing failed"
        );
        assert_eq!(map_pairing_error(&WormholeError::Crypto), "pairing failed");
    }
}
