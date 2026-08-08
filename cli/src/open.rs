//! `docscrying open <code>`: join a serve session over the wormhole pipe and serve
//! the same reader site locally, proxying /api calls through the encrypted
//! pipe. Foreground until Ctrl-C (exit 0).

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use magic_wormhole::{Code, MailboxConnection, Wormhole};

use crate::http::{self, Response};
use crate::protocol::{self, Client, ClientResponse, WormholePipe, reader_loop};
use crate::site;
use crate::OpenArgs;

pub fn run(args: OpenArgs) -> ExitCode {
    let OpenArgs {
        code: code_str,
        rendezvous,
        transit: _transit,
        no_browser,
        port,
    } = args;

    let code: Code = match code_str.parse() {
        Ok(code) => code,
        Err(_) => {
            eprintln!("docscrying: invalid pairing code");
            return ExitCode::from(3);
        }
    };

    let (listener, port) = match http::listen(port) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("docscrying: cannot bind local reader port: {e}");
            return ExitCode::from(1);
        }
    };

    let (req_tx, req_rx) = mpsc::channel();
    let (resp_tx, resp_rx) = mpsc::channel();
    let wormhole_thread = thread::spawn(move || -> Result<(), String> {
        let outcome: Result<Wormhole, String> = async_io::block_on(async {
            let conn = protocol::with_timeout(MailboxConnection::connect(
                protocol::config(&rendezvous),
                code,
                false,
            ))
            .await?;
            protocol::with_timeout(Wormhole::connect(conn)).await
        });
        match outcome {
            Ok(wh) => reader_loop(WormholePipe(wh), req_rx, resp_tx),
            Err(e) => {
                let _ = resp_tx.send(ClientResponse::Ready(Err(e.clone())));
                Err(e)
            }
        }
    });

    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    if ctrlc::set_handler(move || {
        flag.store(true, Ordering::Relaxed);
    })
    .is_err()
    {
        eprintln!("docscrying: cannot install Ctrl-C handler");
        return ExitCode::from(1);
    }

    // Wait for the handshake (10-minute pairing deadline lives in the thread).
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return ExitCode::SUCCESS;
        }
        match resp_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(ClientResponse::Ready(Ok(()))) => break,
            Ok(ClientResponse::Ready(Err(e))) => {
                eprintln!("docscrying: {e}");
                return ExitCode::from(3);
            }
            Ok(other) => {
                eprintln!("docscrying: unexpected reader thread response: {other:?}");
                return ExitCode::from(3);
            }
            Err(RecvTimeoutError::Disconnected) => {
                eprintln!("docscrying: pairing failed");
                return ExitCode::from(3);
            }
            Err(RecvTimeoutError::Timeout) => continue,
        }
    }

    println!("paired (code: {code_str})");
    println!("local reader:  http://localhost:{port}");
    let url = format!("http://localhost:{port}");
    if !no_browser {
        if webbrowser::open(&url).is_err() {
            println!("open this URL in your browser: {url}");
        }
    } else {
        println!("open this URL in your browser: {url}");
    }

    let client = Arc::new(Client::new(req_tx, resp_rx));
    let http_client = client.clone();
    let code_pill = code_str.clone();
    let http_thread = thread::spawn(move || {
        let handler = move |method: &str, path: &str| -> Response {
            site::handle_proxied(method, path, &http_client, &code_pill)
        };
        http::run(listener, handler)
    });

    while !interrupted.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(200));
    }
    let _ = http_thread;
    let _ = wormhole_thread;
    ExitCode::SUCCESS
}
