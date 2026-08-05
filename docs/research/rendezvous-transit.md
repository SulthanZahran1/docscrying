# Rendezvous + Transit Server Research (for scry pairing)

**Status:** Research brief, facts verified against upstream sources (Aug 2026).
**Scope:** Server-side components scry needs for magic-wormhole-style pairing, deployed on the zahranm VPS (Docker + Traefik, `*.zahranm.cloud` subdomains).

Sources: GitHub (`magic-wormhole/magic-wormhole-mailbox-server`, `magic-wormhole/magic-wormhole-transit-relay`, `magic-wormhole/magic-wormhole.rs`, `psanford/wormhole-william`), magic-wormhole readthedocs protocol docs, PyPI, Docker Hub, Traefik docs. Anything not verified is marked explicitly.

---

## 1. Rendezvous / mailbox server

Reference implementation: **`magic-wormhole-mailbox-server` (Python, Twisted)** — the only known mailbox server implementation per the ecosystem docs ("The only known Mailbox Server is magic-wormhole-mailbox-server"). No maintained Rust or Go rendezvous *server* exists; Rust and Go projects implement clients only.

### Deployment facts

- **PyPI:** `magic-wormhole-mailbox-server` latest **0.8.0** (Python 3.10+, MIT). Runs as a Twisted `twist`/`twistd` plugin: `twist wormhole-mailbox [options]`.
- **Protocol:** mailbox protocol **v1** only — version is encoded in the WS path (`/v1`); v1 is "still (as of May, 2026) the only deployed version". Clients connect via **WebSocket** (`ws://host:4000/v1`); store-and-forward so non-simultaneous clients still work; clients auto-reconnect on server reboot.
- **Configuration** (from `src/wormhole_mailbox_server/server_tap.py`):
  - `--port=` endpoint, default `tcp:4000:interface=::` (IPv6 dual-stack). The same port serves the WebSocket — there is **no separate websocket port** (WebSocket is served from `/v1` on the HTTP site on the main port).
  - `--channel-db=` state DB (SQLite), default `relay.sqlite`; `--usage-db=` optional usage SQLite; `--log-fd=`, `--blur-usage=`, `--advertise-version=`, `--motd=`, `--signal-error=`, `--disallow-list` flag.
  - `--websocket-protocol-option=KEY=VALUE` (repeatable, JSON values) — passes options to autobahn's `setProtocolOptions` (e.g. max message/frame sizes). Server sets `autoPingInterval=60, autoPingTimeout=600` by default.
- **Persistence:** SQLite for channel state; a background `TimerService` **prunes nameplates/mailboxes every 5 min; channels expire after 11 min** of inactivity (`CHANNEL_EXPIRATION_TIME = 11*MINUTE`, `EXPIRATION_CHECK_PERIOD = 5*MINUTE`). No operator maintenance needed.
- **AppID:** server does **not restrict app_ids** — clients BIND with any AppID string; isolation is by AppID namespace ("DNSNAME/APPNAME" convention, e.g. `lothar.com/wormhole/text-or-file-xfer`). scry should use its own AppID (e.g. `zahranm.cloud/scry`).
- **Memory/CPU footprint:** not documented anywhere authoritative; Twisted/Python — expect tens of MB RSS, negligible idle CPU (this is our assessment, not upstream-verified). No official benchmark exists.
- **Docker images:** **no official image**. Docker Hub has `leastauthority/magic-wormhole-mailbox` (~5k pulls; runs the upstream for winden.app/Destiny) and `nebulaworks/magic-wormhole-mailbox` (~1.8k pulls) — both community. Repo README documents a trivial Dockerfile (`FROM python:3.11` + pip install + `CMD twist wormhole-mailbox --usage-db=usage.sqlite`) and warns it runs as root; build your own for production.

### Rust/Go alternatives

- **Rust `magic-wormhole` crate 0.8.1** (`magic-wormhole/magic-wormhole.rs`, EUPL-1.2): client library + CLI ("Rusty Wormhole"). **Interoperates with the Python mailbox server over the standard v1 WebSocket protocol** — rendezvous module connects with `async_tungstenite` (ws/wss), default `ws://relay.magic-wormhole.io:4000/v1`; `wss` requires the crate's `tls` feature (rustls/native-tls backends). No server component in the repo.
- **Go `wormhole-william`** (psanford): client library + CLI, interops with the Python CLI/server; default rendezvous `ws://relay.magic-wormhole.io:4000/v1`, default transit `transit.magic-wormhole.io:4001`. Client only.
- **Ecosystem feature matrix** (readthedocs): Python core/reconnect/file-v1 Full; Rust core Full, reconnect Partial, file-v1 Partial, Dilation PoC; Go core Full, file-v1 Full, no Dilation. Haskell: core Full, file-v1 Full (no reconnect).
- Practical note from magic-wormhole issue #243: a user reports running mailbox + transit relay behind **Traefik with TLS successfully** ("reverse proxy through traefik and it works super great").

## 2. Transit relay

Reference implementation: **`magic-wormhole-transit-relay` (Python, Twisted)** — "The only known Transit Relay implementation". Latest PyPI **0.5.0**.

### When it is required

- Both peers behind NAT with no viable direct connection → relay glues two connections with the same handshake token (`please relay $channel for $side`).
- **Browser/WASM clients:** browsers cannot make raw TCP connections; per the protocol spec, "interop between browsers and CLI clients will either require adding WebSocket to CLI, or a relay that is capable of speaking/bridging both". The transit relay supports **TCP and WebSocket transports and can bridge a TCP client to a WS client** ("the relay will also connect two clients using different protocols together"). This makes the relay effectively **mandatory for scry's planned browser-only WASM reader** whenever the peer isn't on the same LAN.
- Not needed if all peers have direct connectivity; the client falls back to relay only after direct hints fail.

### Deployment facts

- Runs as Twisted plugin: `twist transitrelay [ARGS]`. Options: `--port=` (default `tcp:4001`), `--websocket=` (separate WS endpoint, e.g. `tcp:4002`), `--websocket-url=` (advertised WS URL; defaults to `ws://localhost:<port>` — **must be set** behind a proxy), `--usage-db=`, `--log-fd=`, `--blur-usage=`.
- Default TCP port **4001**; WS is a *separate* port, so both must be exposed.
- WS transport details: transit payload over WS uses **binary frames**; framing is still the line/length-prefixed stream inside WS messages; hints advertise `websocket-v1` URLs (`wss://` and `ws://` both supported; a relay on both transports advertises two hints).
- **Footprint:** same class as mailbox server (Twisted/Python); no authoritative numbers.
- **Docker:** no official image. Community: `andreasmueller/wormhole-transit-relay` (0.4.0, ~380 pulls, uses `network_mode: host`), and `ggeorgovassilis/magic-wormhole-transit-relay-docker` (build-from-source Dockerfile, referenced from upstream docs/running.md).
- Rust alternatives: none — no Rust (or Go) transit *relay server* exists; Rust/Go implement the client side (Rust crate has `connect_ws_relay`/`connect_tcp_relay` and parses `relay-v1` hints with `tcp://`, `ws://`, `wss://` URL schemes; ecosystem table: Rust Dilation PoC only).

## 3. Traefik proxying

- **WebSocket/WSS is supported out of the box by Traefik** — "no special configuration is required beyond standard HTTP routing"; Traefik auto-handles the upgrade and preserves `Origin`, `Sec-WebSocket-Key`, `Sec-WebSocket-Version` headers (Traefik docs "Exposing WebSocket Services"; v3.4 user-guide).
- **TLS termination** happens at Traefik (`tls: { certResolver: letsencrypt }` on the router; backend stays plain `http://container:port`). Clients then use `wss://` — the mailbox server and transit relay are plain WS on the backend, so no in-container TLS needed.
- For the **transit relay TCP port** (raw `tcp:4001`), an HTTP router won't do — Traefik **TCP routers** match on `HostSNI`; this is documented Traefik routing capability (used widely for non-HTTP TCP services; we did not verify a concrete transit-relay example, but the Traefik TCP-routing docs cover the pattern). Alternatively skip the proxy for TCP and expose 4001 directly, or rely on WS transit only (browser clients use `websocket-v1` hints anyway; native peers can be told to prefer `wss://` hints too, since the Rust crate accepts `wss` relay URLs).
- **Rate limiting:** Traefik `rateLimit` middleware (token bucket, `average`/`period`/`burst`, source criterion on `X-Forwarded-For` with depth/excludedIPs). Pluggable on HTTP routers — useful for the mailbox endpoint.
- **Subdomain conventions on zahranm VPS** (from existing deployments): `<name>.zahranm.cloud` A record → `72.61.213.90`; router+service entries in `~/hosted_projects/traefik/dynamic.yml`; containers attach to the `web` docker network; DNS via Hostinger (no wildcard). Natural choices: `wormhole.zahranm.cloud` (mailbox), `transit.zahranm.cloud` (relay WS) — both wss on 443.

## 4. Other server-side needs

- **TLS:** terminate at Traefik (Let's Encrypt via certResolver) — no per-container certs. Rust crate needs `tls` feature for `wss://` rendezvous.
- **Nameplate TTL/cleanup:** built into mailbox server (11-min expiry, 5-min prune cycle) — nothing to run externally.
- **Persistence:** SQLite files only (`relay.sqlite` + optional usage DB); mount a volume so nameplates survive container restarts; no Postgres/Redis required.
- **Rate limiting / abuse:** not built into either server (mailbox server has `--disallow-list` only, plus optional `motd`/`signal-error`); add Traefik `rateLimit` middleware in front of the mailbox WS endpoint. `--usage-db` + `--log-fd` provide observability.
- **Health checks:** root path returns `Wormhole Relay\n` text (from `web.py`) — usable as a Traefik/health probe.
- **Unverified items:** official memory/CPU numbers for either server (none published); any maintained Rust/Go server implementations (none found — ecosystem docs list the two Python servers as the *only* implementations).

## Recommendations for scry

1. Deploy `magic-wormhole-mailbox-server` 0.8.0 as a custom Docker image (no official one) on `wormhole.zahranm.cloud`, WS endpoint `wss://wormhole.zahranm.cloud/v1`, SQLite on a volume, Traefik HTTP router + letsencrypt + `rateLimit` middleware.
2. Deploy `magic-wormhole-transit-relay` 0.5.0 with `--websocket=tcp:4002` and `--websocket-url=wss://transit.zahranm.cloud` (required for the browser/WASM client); Traefik HTTP router for the WS port; decide on raw TCP 4001 via Traefik TCP router (`HostSNI`) or omit it if all clients can use `websocket-v1` hints (native Rust clients accept `wss://` relay hints).
3. Use a scry-specific AppID (e.g. `zahranm.cloud/scry`); the Rust `magic-wormhole` 0.8.1 client interoperates with the Python mailbox server via `wss://` (enable its `tls` feature).
4. No extra services needed: no Redis/Postgres, nameplate cleanup is built in.
