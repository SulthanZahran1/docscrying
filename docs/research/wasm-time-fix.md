# wasm-time-fix: upstream status + wasm dependency strategy

Date: 2026-08-05. Scope: magic-wormhole crate 0.8.1 wasm viability for scry's browser client. All claims verified via GitHub API, raw source, local cargo builds, and rust-src; see /tmp/mw/ for the research artifacts.

## 1. Upstream status: NO fix exists

- `magic-wormhole.rs` 0.8.1 (tag `13e1cf7`, 2026-05-07) **and** `main` (HEAD `41d78aed`, 2026-07-22) both still carry the un-gated `use std::time::Instant` at `src/transit.rs:32` (used at `:945`).
- The only "wasm"-related PRs on the repo: `#298` (`WsMessage::text` → `Text`, a *different* fix, already included in 0.8.1) and `#249` (the PR that *introduced* the `std::time::Instant`).
- Zero upstream PRs from the fork author (lucamartinetti). No released version carries the fix.

## 2. The fork commit b695a1d

- Lives on `lucamartinetti/magic-wormhole.rs`, branch `wasm-time-fix` — 1 commit ahead of upstream main, based on a pre-0.8.1 snapshot (`version = "0.8.0-alpha.1"`).
- Diff: gates the `Instant` import and the `Instant::now()` + "wait for better direct connection" block behind `#[cfg(not(target_family = "wasm"))]` in `src/transit.rs` (+7 / −1).
- Its `rendezvous.rs` already matches 0.8.1, so the fork is functionally 0.8.1 + the 8-line wasm gate.

## 3. Nature of the bug: runtime panic, not compile error

- `cargo build --target wasm32-unknown-unknown --features transit,transfer` on 0.8.1 **succeeds**. std's `sys/time/unsupported.rs` makes `Instant::now()` panic at runtime on wasm32-unknown-unknown.
- Proven empirically: a wasm binary calling `Instant::now()` traps with `unreachable` when invoked in node. (Naive node tests "pass" because node never auto-invokes `main`.)
- Upstream CI (`push.yml`, identical in fork and upstream) only *builds* wasm, never *runs* it — nothing catches this. This is why the bug survives despite the crate shipping wasm support.

## 4. The `tls` feature is irrelevant on wasm

- `tls = ["async-tungstenite/async-tls"]`, but `async-tungstenite` sits under `[target.'cfg(not(target_family = "wasm"))'.dependencies]`.
- The wasm path uses `ws_stream_wasm::WsMeta::connect(url, None)` → the browser's native `WebSocket::new(url)` (verified in ws_stream_wasm 0.7.5 source).
- Therefore **wss:// works in the browser via the browser's own TLS, with no crate feature**. `--features transit,transfer,tls` builds fine on wasm (the flag is a no-op there). CI's WASM matrix uses only `transit,transfer`.

## 5. Dependency strategy: git dep pinned by rev

| Option | Assessment |
|---|---|
| Git dep on fork branch (`branch = "wasm-time-fix"`) | Works (wormhole-page precedent) but a branch is mutable — a force-push silently changes your build. |
| **Git dep pinned by `rev = b695a1d...` (recommended)** | Immutable, Cargo.lock-pinned, zero maintenance, matches the proven wormhole-page approach minus the mutability risk. |
| `[patch.crates-io]` pointing at the fork | **Trap**: the fork's version `0.8.0-alpha.1` does not satisfy `^0.8.1`, so Cargo ignores/rejects the patch. Would require a version-bumped fork. |
| Vendored local `[patch]` | Fully reproducible, but makes scry the maintainer of an EUPL-1.2 fork (copy the code in-tree). Heaviest long-term cost. |

**Recommendation:** git dependency pinned by `rev = b695a1d` (immutable, Cargo.lock-pinned, zero maintenance, wormhole-page precedent). Revisit when/if upstream merges a wasm fix.

## Notes

- wormhole-page itself uses the git-dep-on-fork-branch form: `git = "...", branch = "wasm-time-fix", default-features = false, features = ["transit", "transfer"]`.
- scry should do the same features set (`transit,transfer`, default-features off) for the wasm client, and pin by rev.
- Upstream tracking: revisit after any magic-wormhole.rs release > 0.8.1; check transit.rs:32 for the cfg gate.
