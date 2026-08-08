//! The embedded reader site: one self-contained HTML page (inline CSS + JS)
//! served by both `serve` (direct file access) and `open` (proxied over the
//! wormhole pipe). Placeholders __SCRY_REPO__ / __SCRY_CODE__ are substituted
//! per mode.

use crate::http::Response;
use crate::index::{fmt_size, Doc, Index};
use crate::protocol::{fetch_doc, Client, GetResult};

const SITE_HTML: &str = include_str!("site.html");

fn page(repo: &str, code: &str) -> Response {
    let html = SITE_HTML
        .replace("__SCRY_REPO__", &escape_attr(repo))
        .replace("__SCRY_CODE__", &escape_attr(code));
    Response::html(html)
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Serve mode: read straight from the index on this machine.
/// `code` is what the header pill shows ("local" for local serve, "hosted"
/// for hosted mode — no pairing code exists in either).
pub fn handle_direct(method: &str, path: &str, index: &Index, repo: &str, code: &str) -> Response {
    if method != "GET" {
        return Response::text(400, "only GET is supported");
    }
    let path = path.split('?').next().unwrap_or(path);
    match path {
        "/" | "/index.html" => page(repo, code),
        "/api/tree" => Response::json(
            200,
            serde_json::to_vec(&serde_json::json!({"docs": index.docs}))
                .unwrap_or_else(|_| b"{}".to_vec()),
        ),
        p if p.starts_with("/api/doc/") => {
            let id = p.trim_start_matches("/api/doc/");
            let Ok(id) = id.parse::<u64>() else {
                return Response::json(400, b"{\"error\":\"bad doc id\"}".to_vec());
            };
            let doc = fetch_doc(index, id);
            match doc {
                doc if doc.status == 200 => Response {
                    status: 200,
                    content_type: doc
                        .content_type
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                    body: doc.body.unwrap_or_default(),
                },
                doc => Response::json(
                    doc.status,
                    serde_json::to_vec(&serde_json::json!({"error": doc.error}))
                        .unwrap_or_else(|_| b"{}".to_vec()),
                ),
            }
        }
        _ => Response::text(404, "not found"),
    }
}

/// Open mode: proxy /api/* over the wormhole pipe.
pub fn handle_proxied(method: &str, path: &str, client: &Client, code: &str) -> Response {
    if method != "GET" {
        return Response::text(400, "only GET is supported");
    }
    let path = path.split('?').next().unwrap_or(path);
    match path {
        "/" | "/index.html" => page(code, code),
        "/api/tree" => match client.list() {
            Ok(docs) => Response::json(
                200,
                serde_json::to_vec(&serde_json::json!({"docs": docs}))
                    .unwrap_or_else(|_| b"{}".to_vec()),
            ),
            Err(e) => Response::json(
                502,
                serde_json::to_vec(&serde_json::json!({"error": e})).unwrap_or_default(),
            ),
        },
        p if p.starts_with("/api/doc/") => {
            let id = p.trim_start_matches("/api/doc/");
            let Ok(id) = id.parse::<u64>() else {
                return Response::json(400, b"{\"error\":\"bad doc id\"}".to_vec());
            };
            match client.get(id) {
                Ok(GetResult::Ok {
                    content_type, body, ..
                }) => Response {
                    status: 200,
                    content_type,
                    body,
                },
                Ok(GetResult::Err { status, message }) => Response::json(
                    status,
                    serde_json::to_vec(&serde_json::json!({"error": message})).unwrap_or_default(),
                ),
                Err(e) => Response::json(
                    502,
                    serde_json::to_vec(&serde_json::json!({"error": e})).unwrap_or_default(),
                ),
            }
        }
        _ => Response::text(404, "not found"),
    }
}

pub fn _doc_sizes(docs: &[Doc]) -> String {
    let total: u64 = docs.iter().map(|d| d.size).sum();
    fmt_size(total)
}
