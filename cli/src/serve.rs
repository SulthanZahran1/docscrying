//! `docscrying serve`: index a directory (or GitHub repo), serve the reader
//! site, and either accept paired readers over the wormhole pipe (relay-v1)
//! or run hosted (always-on HTTP, no pairing) for reverse-proxy deployments.

use std::process::ExitCode;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use magic_wormhole::{MailboxConnection, Password, Wormhole};

use crate::github;
use crate::http::{self, Response};
use crate::index::{fmt_size, index_dir, Index};
use crate::protocol::{self, server_loop, WormholePipe};
use crate::site;
use crate::ServeArgs;

pub fn run(args: ServeArgs) -> ExitCode {
    let ServeArgs {
        dir,
        port,
        rendezvous,
        transit: _transit,
        once,
        hosted,
        bind,
        token,
        refresh,
    } = args;
    let dir = dir.unwrap_or_else(|| ".".to_string());
    let (index, repo, source_note, github_spec) = if let Some(spec) = dir.strip_prefix("github:") {
        // GitHub source: download the tarball at the resolved commit, index
        // the extracted tree exactly like a local directory.
        let src = match github::fetch(spec, token.as_deref()) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("docscrying: {e}");
                return ExitCode::from(1);
            }
        };
        let index = match index_dir(&src.dir) {
            Ok(index) => index,
            Err(e) => {
                eprintln!("docscrying: {e}");
                return ExitCode::from(1);
            }
        };
        let note = format!(
            "github:{} @ {} ({})",
            src.display,
            &src.sha[..src.sha.len().min(12)],
            src.reference.as_deref().unwrap_or("default branch")
        );
        (index, src.display, note, Some(spec.to_string()))
    } else {
        let path = std::path::PathBuf::from(&dir);
        let index = match index_dir(&path) {
            Ok(index) => index,
            Err(e) => {
                eprintln!("docscrying: {e}");
                return ExitCode::from(1);
            }
        };
        let repo = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        (index, repo, String::new(), None)
    };
    let total: u64 = index.docs.iter().map(|d| d.size).sum();
    println!("indexed {} docs ({})", index.docs.len(), fmt_size(total));
    if !source_note.is_empty() {
        println!("source:        {source_note}");
    }

    if hosted {
        return run_hosted(index, repo, port, &bind, github_spec, token, refresh);
    }

    let (listener, port) = match http::listen(port) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("docscrying: cannot bind local reader port: {e}");
            return ExitCode::from(1);
        }
    };
    println!("local reader:  http://localhost:{port}");
    println!("pairing page:  https://wormhole.zahranm.cloud");

    let site_index = index.clone();
    let http_thread = thread::spawn(move || {
        let handler = move |method: &str, path: &str| -> Response {
            site::handle_direct(method, path, &site_index, &repo, "local")
        };
        http::run(listener, handler)
    });

    let mut reader = 1u64;
    loop {
        let outcome = pair_one(&rendezvous, &index, reader);
        match outcome {
            PairOutcome::Left => println!("reader {reader} left"),
            PairOutcome::Failed(message) => eprintln!("docscrying: {message}"),
        }
        reader += 1;
        if once {
            break;
        }
    }
    let _ = http_thread;
    ExitCode::SUCCESS
}

/// Hosted mode: bind the given address, serve the reader over plain HTTP
/// forever (no wormhole pairing). Optional `--refresh N` re-fetches and
/// re-indexes a GitHub source every N minutes, swapping the index live.
fn run_hosted(
    index: Index,
    repo: String,
    port: u16,
    bind: &str,
    github_spec: Option<String>,
    token: Option<String>,
    refresh: Option<u64>,
) -> ExitCode {
    let listener = match std::net::TcpListener::bind((bind, port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("docscrying: cannot bind {bind}:{port}: {e}");
            return ExitCode::from(1);
        }
    };
    println!("hosted reader: http://{bind}:{port} (no pairing)");

    let shared: Arc<RwLock<Index>> = Arc::new(RwLock::new(index));

    // Refresh thread: re-fetch the GitHub source, re-index, swap live.
    if let (Some(spec), Some(mins)) = (github_spec, refresh) {
        let shared = shared.clone();
        let repo_name = repo.clone();
        let token = token.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(mins * 60));
            match github::fetch(&spec, token.as_deref()) {
                Ok(src) => match index_dir(&src.dir) {
                    Ok(fresh) => {
                        let n = fresh.docs.len();
                        let total: u64 = fresh.docs.iter().map(|d| d.size).sum();
                        let sha = &src.sha[..src.sha.len().min(12)];
                        *shared.write().unwrap() = fresh;
                        println!(
                            "refreshed {repo_name} @ {sha}: {n} docs ({})",
                            fmt_size(total)
                        );
                    }
                    Err(e) => eprintln!("docscrying: refresh re-index failed: {e}"),
                },
                Err(e) => eprintln!("docscrying: refresh fetch failed: {e}"),
            }
        });
    }

    let site_index = shared.clone();
    let handler = move |method: &str, path: &str| -> Response {
        let index = site_index.read().unwrap();
        site::handle_direct(method, path, &index, &repo, "hosted")
    };
    http::run(listener, handler);
    ExitCode::SUCCESS
}

enum PairOutcome {
    Left,
    Failed(String),
}

/// One pairing round: allocate a fresh nameplate, print the code, run the
/// session until the reader leaves (EOF on the pipe).
fn pair_one(rendezvous: &str, index: &Index, reader: u64) -> PairOutcome {
    let outcome = async_io::block_on(async {
        let password = random_password()
            .parse::<Password>()
            .map_err(|e| format!("internal error: {e}"))?;
        let conn = protocol::with_timeout(MailboxConnection::create_with_password(
            protocol::config(rendezvous),
            password,
        ))
        .await?;
        let code = conn.code().to_string();
        println!("pairing code:  {code}");
        let wh = protocol::with_timeout(Wormhole::connect(conn)).await?;
        Ok::<_, String>((code, wh))
    });
    let (code, wh) = match outcome {
        Ok(ok) => ok,
        Err(e) => return PairOutcome::Failed(e),
    };
    println!("reader {reader} connected ({code})");
    let _ = server_loop(WormholePipe(wh), index);
    PairOutcome::Left
}

/// Random lowercase-hex password, shaped like the codes the mailbox expects
/// (letters, digits, hyphens only).
fn random_password() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9)
        ^ std::process::id() as u64;
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let hex = |n: u64| format!("{n:08x}");
    format!("docscrying-{}-{}", hex(next()), hex(next()))
}
