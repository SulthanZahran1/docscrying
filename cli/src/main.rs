//! docscrying: index every doc-like file in a codebase, serve a local reader site,
//! pair any other machine with a wormhole code.
//!
//! Two commands:
//! - `docscrying serve [dir]` runs the indexer, serves the reader site on
//!   127.0.0.1, and accepts readers over a magic-wormhole pipe (relay-v1).
//! - `docscrying open <code>` joins an existing serve session over the pipe and
//!   serves the same reader site locally, proxying /api calls through the
//!   encrypted wormhole.

mod http;
mod index;
mod open;
mod protocol;
mod serve;
mod site;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

pub const APP_ID: &str = "zahranm.cloud/docscrying";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_RENDEZVOUS: &str = "wss://wormhole.zahranm.cloud/v1";
pub const DEFAULT_TRANSIT: &str = "wss://transit.zahranm.cloud";
pub const DEFAULT_PORT: u16 = 8765;
pub const PAIRING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// docscrying: read every doc in a codebase from anywhere, via a pairing code.
#[derive(Parser, Debug)]
#[command(name = "docscrying", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Index a directory and serve the reader site, accepting paired readers
    Serve(ServeArgs),
    /// Join a serve session with a pairing code and serve it locally
    Open(OpenArgs),
}

#[derive(clap::Args, Debug)]
struct ServeArgs {
    /// Directory to index (defaults to the current directory)
    dir: Option<PathBuf>,
    /// Local reader port (falls back to the next free port if busy)
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Magic-wormhole rendezvous server
    #[arg(long, default_value = DEFAULT_RENDEZVOUS)]
    rendezvous: String,
    /// Magic-wormhole transit relay (reserved; the relay-v1 pipe never uses transit)
    #[arg(long, default_value = DEFAULT_TRANSIT)]
    transit: String,
    /// Serve a single pairing then exit
    #[arg(long)]
    once: bool,
}

#[derive(clap::Args, Debug)]
struct OpenArgs {
    /// The pairing code printed by `scry serve`
    code: String,
    /// Magic-wormhole rendezvous server
    #[arg(long, default_value = DEFAULT_RENDEZVOUS)]
    rendezvous: String,
    /// Magic-wormhole transit relay (reserved; the relay-v1 pipe never uses transit)
    #[arg(long, default_value = DEFAULT_TRANSIT)]
    transit: String,
    /// Print the local URL instead of opening a browser
    #[arg(long)]
    no_browser: bool,
    /// Local reader port (falls back to the next free port if busy)
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve::run(args),
        Command::Open(args) => open::run(args),
    }
}
