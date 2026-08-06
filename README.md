# docscrying

Point it at any codebase, read every scattered doc (md/html/rst/adoc/txt/pdf/docx) in a local website, pair from anywhere with a magic-wormhole-style code. Rust.

## What's here

- `cli/` — the native CLI: `docscrying serve [dir]` (index + local reader site + wormhole server) and `docscrying open <code>` (pair and read through the encrypted pipe). Single binary, localhost-only reader, relay-v1 protocol.
- `wasm/` — the browser client: wasm32 magic-wormhole pairing + relay-v1, plus the reader page (pairing page entry point). Build with `wasm-pack build --target web` (outputs `pkg/`).

## relay-v1

Both clients speak the same wire protocol over the wormhole pipe: hello-first with exact version match, `list` -> whole `tree` one shot, `get` -> `data` control frame + exactly one raw body record (empty on error), strict alternation, EOF = close. Tree immutable per session. 25 MB cap (413).

## Pairing infra

Rendezvous + transit run at wormhole.zahranm.cloud / transit.zahranm.cloud (deployed from `~/hosted_projects/scry-pairing`). CLI defaults point there; both clients accept `--rendezvous` overrides.
