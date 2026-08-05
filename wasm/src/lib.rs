//! scry reader WASM client (ticket #12).
//!
//! Pairing via magic-wormhole (fork-pinned, wasm-safe), then the relay-v1
//! protocol rides entirely on the wormhole pipe: strict alternation, one
//! in-flight message, no ids, no transit. EOF from the peer closes the session.
//!
//! Exported surface (async wasm-bindgen):
//!   pair(code) -> Result<(), JsValue>      handshake: wait server hello, reply, verify
//!   list_docs() -> Result<JsValue, JsValue>  [{id,path,kind,size,mtime}, ...]
//!   get_doc(id) -> Result<JsValue, JsValue>  {status, content_type, bytes|error}
//!   PROTOCOL_VERSION: u32 = 1

use std::cell::RefCell;
use std::future::Future;

use js_sys::Reflect;
use magic_wormhole::transfer::AppVersion;
use magic_wormhole::{AppConfig, AppID, Code, MailboxConnection, Wormhole, WormholeError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasm_bindgen::prelude::*;

/// relay-v1 protocol version spoken by this client.
const PROTOCOL_VERSION: u32 = 1;

/// Exported protocol version accessor.
#[wasm_bindgen]
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// Hard cap enforced by the relay server. The reader pre-checks the tree and
/// never requests docs at or above this size.
const MAX_DOC_SIZE: u64 = 25 * 1024 * 1024;

/// Pairing deadline, matching the CLI's 60s timeout (decision #6).
const PAIRING_TIMEOUT_MS: u32 = 60_000;

const APP_ID: &str = "zahranm.cloud/scry";
const RENDEZVOUS_URL: &str = "wss://wormhole.zahranm.cloud/v1";

thread_local! {
    /// The paired wormhole pipe, shared by list/get. One session at a time;
    /// re-pairing replaces it.
    static WORMHOLE: RefCell<Option<Wormhole>> = const { RefCell::new(None) };
    /// One-in-flight guard: relay-v1 is strictly alternating, so a second
    /// concurrent call would desync the pipe.
    static BUSY: RefCell<bool> = const { RefCell::new(false) };
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

/// Wasm-safe sleep (setTimeout via js_sys; std::time::Instant/thread::sleep
/// are unavailable on wasm32-unknown-unknown).
async fn sleep(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .expect("no window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
            .expect("setTimeout failed");
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn app_config() -> AppConfig<AppVersion> {
    magic_wormhole::transfer::APP_CONFIG
        .clone()
        .id(AppID::new(APP_ID))
        .rendezvous_url(std::borrow::Cow::Owned(RENDEZVOUS_URL.to_string()))
}

/// Map a wormhole error to a human-readable JS string.
fn friendly(e: &WormholeError) -> String {
    match e {
        WormholeError::UnclaimedNameplate(_) => {
            "Wrong or expired code: no session is waiting on this code".to_string()
        }
        WormholeError::CodeInvalid(_) => {
            "Malformed code: expected nameplate-password (e.g. 7-crossover-clockwork)".to_string()
        }
        WormholeError::PakeFailed => {
            "Pairing failed: the code does not match the sender's session".to_string()
        }
        WormholeError::Crypto => "Pairing failed: message decryption error".to_string(),
        WormholeError::ServerError(_) => format!("Rendezvous unreachable: {e}"),
        other => other.to_string(),
    }
}

fn wh_err(e: WormholeError) -> JsValue {
    JsValue::from_str(&friendly(&e))
}

/// Serialize a JS error (protocol JSON layer) to a JsValue string.
fn json_err(ctx: &str, e: serde_json::Error) -> JsValue {
    JsValue::from_str(&format!("{ctx}: {e}"))
}

/// Wrap an operation in the one-in-flight guard.
async fn guarded<F, T>(op: F) -> Result<T, JsValue>
where
    F: Future<Output = Result<T, JsValue>>,
{
    if BUSY.with(|b| *b.borrow()) {
        return Err(JsValue::from_str(
            "A request is already in flight; wait for it to finish",
        ));
    }
    BUSY.with(|b| *b.borrow_mut() = true);
    let result = op.await;
    BUSY.with(|b| *b.borrow_mut() = false);
    result
}

// ---------------------------------------------------------------------------
// relay-v1 wire messages
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct HelloMsg {
    #[serde(rename = "type")]
    ty: String,
    v: u32,
    role: String,
}

#[derive(Deserialize, Serialize)]
struct DocMeta {
    id: u32,
    path: String,
    kind: String,
    size: u64,
    /// Unix seconds; matches the CLI index's u64 mtime on the wire.
    mtime: u64,
}

#[derive(Deserialize)]
struct TreeMsg {
    #[serde(rename = "type")]
    ty: String,
    docs: Vec<DocMeta>,
}

#[derive(Deserialize)]
struct DataMsg {
    #[serde(rename = "type")]
    ty: String,
    status: u16,
    content_type: Option<String>,
    size: Option<u64>,
}

// ---------------------------------------------------------------------------
// Exported API
// ---------------------------------------------------------------------------

/// Pair with a scry session: join the code's mailbox, run the wormhole key
/// exchange, then complete the relay-v1 hello handshake (wait for the server
/// hello, reply as reader, verify the version). On success the pipe is kept
/// open for list_docs/get_doc. Bounded by a 60s deadline so an expired or
/// mistyped code reports an error instead of hanging forever.
#[wasm_bindgen]
pub async fn pair(code: &str) -> Result<(), JsValue> {
    if BUSY.with(|b| *b.borrow()) {
        return Err(JsValue::from_str(
            "A request is already in flight; wait for it to finish",
        ));
    }
    BUSY.with(|b| *b.borrow_mut() = true);
    let result = match futures::future::select(
        Box::pin(pair_inner(code)),
        Box::pin(async {
            sleep(PAIRING_TIMEOUT_MS).await;
            Err::<(), JsValue>(JsValue::from_str(
                "Pairing timed out after 60s: the code may be wrong or expired",
            ))
        }),
    )
    .await
    {
        futures::future::Either::Left((r, _)) => r,
        futures::future::Either::Right((r, _)) => r,
    };
    BUSY.with(|b| *b.borrow_mut() = false);
    result
}

async fn pair_inner(code: &str) -> Result<(), JsValue> {
    log(&format!("[scry] pairing with code {}", code.split('-').next().unwrap_or("?")));
    let parsed: Code = code.parse().map_err(|_| {
        JsValue::from_str("Malformed code: expected nameplate-password (e.g. 7-crossover-clockwork)")
    })?;

    let mailbox = MailboxConnection::connect(app_config(), parsed, false)
        .await
        .map_err(wh_err)?;
    log("[scry] mailbox joined, running key exchange");

    let mut wormhole = Wormhole::connect(mailbox).await.map_err(wh_err)?;
    log("[scry] key exchange complete");

    // relay-v1: the server speaks first.
    let hello: HelloMsg = wormhole
        .receive_json()
        .await
        .map_err(wh_err)?
        .map_err(|e| json_err("Bad server hello", e))?;

    if hello.ty != "hello" || hello.role != "server" {
        return Err(JsValue::from_str(&format!(
            "Unexpected first message from server (type={}, role={})",
            hello.ty, hello.role
        )));
    }
    if hello.v != PROTOCOL_VERSION {
        return Err(JsValue::from_str(&format!(
            "Protocol version mismatch: server speaks v{}, client speaks v{PROTOCOL_VERSION}",
            hello.v
        )));
    }

    wormhole
        .send_json(&json!({"type": "hello", "v": PROTOCOL_VERSION, "role": "reader"}))
        .await
        .map_err(wh_err)?;
    log("[scry] paired, relay-v1 handshake complete");

    WORMHOLE.with(|c| *c.borrow_mut() = Some(wormhole));
    Ok(())
}

/// Request the document tree. Returns a JS array of
/// {id, path, kind, size, mtime}. The tree is immutable for the session.
#[wasm_bindgen]
pub async fn list_docs() -> Result<JsValue, JsValue> {
    guarded(list_inner()).await
}

async fn list_inner() -> Result<JsValue, JsValue> {
    let mut wormhole = WORMHOLE
        .with(|c| c.borrow_mut().take())
        .ok_or_else(|| JsValue::from_str("Not paired"))?;
    let result = async {
        wormhole
            .send_json(&json!({"type": "list"}))
            .await
            .map_err(wh_err)?;
        let tree: TreeMsg = wormhole
            .receive_json()
            .await
            .map_err(wh_err)?
            .map_err(|e| json_err("Bad tree from server", e))?;
        serde_wasm_bindgen::to_value(&tree.docs)
            .map_err(|e| JsValue::from_str(&format!("Result serialization failed: {e}")))
    }
    .await;
    WORMHOLE.with(|c| *c.borrow_mut() = Some(wormhole));
    result
}

/// Fetch one doc by id. Returns {status, content_type, bytes} where bytes is
/// a Uint8Array on 200; on non-200 the body (if any) is exposed as `error`.
/// Callers pre-check size against the 25 MB cap using the tree.
#[wasm_bindgen]
pub async fn get_doc(id: u32) -> Result<JsValue, JsValue> {
    guarded(get_inner(id)).await
}

async fn get_inner(id: u32) -> Result<JsValue, JsValue> {
    let mut wormhole = WORMHOLE
        .with(|c| c.borrow_mut().take())
        .ok_or_else(|| JsValue::from_str("Not paired"))?;
    let result = async {
        wormhole
            .send_json(&json!({"type": "get", "id": id}))
            .await
            .map_err(wh_err)?;
        let data: DataMsg = wormhole
            .receive_json()
            .await
            .map_err(wh_err)?
            .map_err(|e| json_err("Bad data message from server", e))?;

        // Strict alternation: every get is answered by exactly one data message
        // followed by one body message (empty for error statuses).
        let body: Vec<u8> = wormhole.receive().await.map_err(wh_err)?;

        let content_type = data.content_type.unwrap_or_default();
        let result = js_sys::Object::new();
        Reflect::set(&result, &"status".into(), &JsValue::from_f64(data.status as f64))?;
        Reflect::set(&result, &"content_type".into(), &JsValue::from_str(&content_type))?;
        if data.status == 200 {
            let bytes = js_sys::Uint8Array::new_with_length(body.len() as u32);
            bytes.copy_from(&body);
            Reflect::set(&result, &"bytes".into(), &bytes)?;
        } else if !body.is_empty() {
            Reflect::set(
                &result,
                &"error".into(),
                &JsValue::from_str(&String::from_utf8_lossy(&body)),
            )?;
        }
        Ok(result.into())
    }
    .await;
    WORMHOLE.with(|c| *c.borrow_mut() = Some(wormhole));
    result
}

/// Expose the size cap so the page can show the "too large" card locally.
#[wasm_bindgen]
pub fn max_doc_size() -> u64 {
    MAX_DOC_SIZE
}
