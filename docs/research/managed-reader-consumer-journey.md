# Managed reader consumer journey research

**Ticket:** [#15 — consumer onboarding and accessibility for the managed reader](https://github.com/SulthanZahran1/docscrying/issues/15)<br>
**Research branch:** `research/managed-reader-consumer-journey`<br>
**Source snapshot:** `503d55e52cf6348081829d1697bfc9c1ed593947` (the local `main` before this branch)<br>
**Live observation:** `https://wormhole.zahranm.cloud/reader/`, observed 2026-08-15T14:47Z–14:49Z.<br>
**Scope:** research only. No CLI, protocol, deployment, map, or dependent issue was changed.

This note answers the research question in issue #15.[6]

## Executive recommendation

Treat the managed URL as a small, explicit state machine rather than a generic form: **explain → enter code → pairing → paired/loading → reading → recover**. Keep the one-time wormhole code, no-account model, relay-v1 contract, and stable URL unchanged.[2][4][7] The narrow polish pass should add plain-language producer instructions and one-time/expiry expectations, a programmatically announced progress/success/error status, a retry path that preserves the entered code, keyboard-operable tree/drawer controls with managed focus, reduced-motion rules, and an explicit unsupported-browser message. The next decision ticket (#16) should lock those states and acceptance checks before implementation.

## Method and evidence boundaries

I inspected the deployed page in a browser and with `curl`, tried one malformed code (`not-a-code`) and one valid-shaped but unclaimed code (`1-crossover-clockwork`), inspected the current source at the snapshot above, read the issue/map decisions #1, #7, #12, #14, and #16, and consulted W3C WAI WCAG 2.2 guidance plus MDN platform guidance. The live page returned HTTP 200 and rendered the pairing form; the browser accessibility snapshot exposed a heading, textbox, and pair button. The malformed and unclaimed-code messages below are therefore observed behavior, not predictions.[1]

The live page is the WASM pairing entry point. Issue #14's separately described hosted mode is a different no-pairing deployment shape using the embedded CLI site; do not merge its hosted/password journey into this ticket's one-time-code journey.[5][19][20]

## Current journey, separated by provenance

### 1. First run and code entry

**Observed now**

- The live first screen is a centered dark card with the `docscrying` heading, the instruction “Enter the pairing code from the box serving the docs”, a text input whose placeholder is `7-crossover-clockwork`, and a `pair` button. There is no visible explanation of what the reader is, that the code is one-time, how long it may remain valid, or what the sender should tell the recipient.[1]
- The source has the same minimal structure: `wasm/reader.html:485-492` [17]. The input has a placeholder but no explicit `<label>`; the live DOM inspection also found no `aria-label` or `aria-labelledby` on the pairing input. The visible instruction is helpful, but it is not a durable field label.[1][9]
- The approved prototype direction is still the two-pane IDE structure with an Editorial-style reading column and a mobile drawer, not a final onboarding/content decision.[3][4] The map explicitly leaves first-screen hierarchy and terminology unspecified.[2]

**Recommendation for #16**

Make the first screen answer three questions before the form: “What is this?”, “Where do I get the code?”, and “What happens after I pair?” Use a visible label such as “One-time pairing code”, retain the example as supporting text, and state that the sender must keep the serving process/session available. Do not add accounts, stable shares, or a second pairing system.

### 2. Pairing progress, failure, and retry

**Observed now**

- Clicking `pair` disables the button and removes the previous error while `pair(code)` and `list_docs()` run; it re-enables the button in `finally`: `wasm/reader.html:564-586` [17]. There is no visible “pairing…”, phase indicator, timeout countdown, or explanation of what the browser is waiting for.
- The live malformed-code attempt displayed `Malformed code: expected nameplate-password (e.g. 7-crossover-clockwork)`. The valid-shaped but unclaimed attempt displayed `Wrong or expired code: no session is waiting on this code`. In both cases the code stayed in the input and the pair button remained available for another attempt.[1] The error renderer is a plain `div` with no `role` or `aria-live` in the live DOM inspection.[1][17]
- The WASM client distinguishes malformed, wrong/expired, PAKE/decryption failure, rendezvous failure, and a 60-second timeout: `wasm/src/lib.rs:79-95` and `164-193` [18]. The issue #12 implementation note records this as the intended browser behavior.[4]
- The native `docscrying open` path has its own local-reader handshake and browser-open flow: `cli/src/open.rs:76-108` [21]. That is not the managed page's browser pairing state and should not be presented as evidence that the managed page exposes the same progress UI.
- After pairing and listing, the app changes to the reader and writes `paired via wormhole` into the status bar: `wasm/reader.html:589-595` [17]. There is no separate, announced success step. The one-time/no-account constraint is a product boundary from #1, not a reason to hide the journey state.[2]

**Recommendation for #16**

Define a small contract with stable copy: `Ready for code`, `Pairing…`, `Paired; loading document list…`, `Connected`, `Invalid code`, `Code expired or sender unavailable`, `Connection failed`, `Session ended`, and `Try again`. Announce non-focus-changing updates through a status region and place focus only when the user needs to act. Preserve the code on recoverable failure, keep retry explicit, and say whether retrying is safe after a timeout. W3C’s status-message guidance specifically calls for important changes that do not take focus to be programmatically available to assistive technology.[10]

### 3. Reading, loading, and errors

**Observed now**

- The tree is fetched as one immutable session list: `wasm/src/lib.rs:244-270` [18]. The client pre-checks the 25 MB cap and displays a non-download card for an oversized document, then shows a skeleton while fetching ordinary documents: `wasm/reader.html:732-752` [17].
- Non-200 document responses become a generic card containing the server error; exceptions become `Fetch failed` with the exception string: `wasm/reader.html:756-790` [17]. There is no document-level retry button, and the WASM page does not map a broken pipe to a specific “session ended” recovery action.
- The native embedded site has more specific direct/proxied outcomes, including 413, 404, 500, and a disconnected message (“The pairing pipe is closed … Re-run docscrying open with a fresh code”): `cli/src/site.html:808-845` [19]. That copy is useful evidence, but it is not proof that the managed WASM page currently exposes the same state.
- The server’s contract is explicit about 404, 413, and 500 read outcomes and a body record for every `get`: `cli/src/protocol.rs:130-167` and `172-214` [23]. The indexer lists supported kinds and the 25 MB cap at `cli/src/index.rs:10-11` and `109-143` [22].

**Recommendation for #16**

Separate “the code did not pair”, “the session disappeared after pairing”, “this document is too large”, and “this document could not be read”. Each should have a human-readable cause, a next action, and a retry/re-pair affordance where technically meaningful. Keep the existing content limits and rendering scope; this is recovery copy and state handling, not a new document backend.

### 4. Mobile layout and touch

**Observed/source-backed behavior**

- At widths up to 767px the reader becomes a single column; the sidebar is a fixed drawer up to `min(320px, 84vw)`, opened by the TREE button and closed by a close button, scrim, or document selection: `wasm/reader.html:446-479` and `804-826` [17]. The mobile rules enlarge the pair/search controls, add larger tree-row padding, hide nonessential header metadata, and make tables/code horizontally scrollable inside the content area.[17]
- The issue #14 live gate reports a 390×844 phone viewport with drawer open/select/auto-close and no horizontal overflow for the hosted embedded reader. That is useful precedent, but it is not an independent test of the current WASM page at `/reader/`.[5]
- W3C’s Reflow criterion uses 320 CSS pixels as the narrow reflow reference and requires avoiding loss of information/functionality or two-dimensional scrolling, subject to its stated exceptions.[11] Target Size (Minimum) sets a 24×24 CSS-pixel pointer target or sufficient spacing as the AA baseline.[12]

**Recommendation for #16**

Set the acceptance viewport to at least 320 CSS px wide, test zoom/reflow as well as a 390×844 phone, and retain internal scrolling only for genuinely wide code/data content. Verify every visible control and row against the 24×24 target baseline. Define drawer behavior: focus enters the drawer when opened, Escape closes it, focus returns to TREE, and the scrim is not the only way out.

### 5. Keyboard and screen-reader behavior

**Observed/source-backed behavior**

- The WASM document rows and directory nodes are generated as `div`s and receive click handling only: `wasm/reader.html:804-817` [17]. There is no keydown handler for tree rows or directory expansion. The native CLI site is slightly better for document rows because it adds `tabindex="0"` and handles Enter, but it still does not establish a complete keyboard tree contract: `cli/src/site.html:602-625` and `863-867` [19].
- The deployed pairing form has a native textbox and button, but the input has no explicit label, the error has no live-region semantics, and the TREE/drawer buttons expose labels but not expanded state or controlled-region relationships. The live DOM inspection confirmed `pair-error` had no `role` or `aria-live`.[1]
- WCAG 2.1.1 requires pointer-operated functionality to have a keyboard equivalent.[8] WCAG 3.3.2 calls for labels or instructions when input is required.[9] WCAG 4.1.3 calls for status messages to be programmatically determinable without taking focus.[10]

**Recommendation for #16**

Require semantic buttons/links or equivalent roving-tabindex behavior for directory and document navigation; Enter/Space activation; visible focus; Escape for the drawer; focus return; an explicit input label; `aria-expanded`/`aria-controls` for the drawer; and a `role="status"` or `aria-live` region for progress, success, and recoverable errors. Test with keyboard-only navigation and at least one screen reader rather than treating the presence of `aria-label` on TREE as sufficient.

### 6. Motion and reduced motion

**Observed/source-backed behavior**

- The reader uses animated grid/drawer transitions and switch/caret transitions, including the mobile drawer transform at `wasm/reader.html:101-107`, `225-230`, `384-393`, and `446-458` [17]. Neither the live stylesheet inspection nor the current source contains a `prefers-reduced-motion` rule.[1][17]
- MDN documents `prefers-reduced-motion: reduce` as the way to detect a device-level reduced-motion preference and reports broad browser availability since January 2020.[13]

**Recommendation for #16**

Add reduced-motion acceptance: when the preference is `reduce`, remove or sharply shorten drawer/grid/caret transitions and avoid motion as the only state cue. Keep visible focus and selected-state styling independent of animation.

### 7. Minimum browser support surface

**Observed/source-backed behavior**

- The managed page is an ES module that imports generated WASM glue: `wasm/reader.html:539-540` [17]. The WASM crate is a `cdylib` using wasm-bindgen, js-sys, wasm-bindgen-futures, and web-sys: `wasm/Cargo.toml:8-23` [24]. The client uses a secure WebSocket rendezvous URL and a 60-second async timeout: `wasm/src/lib.rs:32-39` and `164-193` [18].
- There is no `nomodule` fallback or unsupported-browser message in the page. If module/WASM initialization cannot run, the source does not provide a user-facing alternative before the static pairing view; `wasm/reader.html:828-829` only initializes and logs the protocol version on success.[17]
- MDN describes JavaScript modules and WebAssembly as browser platform features and marks WebSocket as widely available; these references support a feature-based contract, not a claim about a particular version cutoff.[14][15][16]

**Recommendation for #16**

Support current evergreen Chrome/Edge, Firefox, and Safari-class browsers that provide ES modules, WebAssembly, `fetch`/Promises, and secure WebSocket. Do not promise legacy browsers without a tested fallback. Add feature-detection failure copy with a browser-update/alternate-browser suggestion, and test the supported set at normal zoom, 200%/400% zoom, 320 CSS px, keyboard-only, reduced motion, and a disrupted network/session.

## Evidence table

| Area | Observed current behavior | Evidence and external constraint | Research recommendation |
|---|---|---|---|
| First run | Minimal heading, one instruction, placeholder-only code field, pair button; no one-time/expiry explanation.[1] | `wasm/reader.html:485-492` [17]; WCAG labels/instructions.[9] | Add visible label, sender/code instructions, one-time/expiry expectation, and next-step copy. |
| Pairing | Button disables during pair/list; no visible progress; malformed and unclaimed code errors observed; retry keeps code.[1] | `wasm/reader.html:564-586` [17]; `wasm/src/lib.rs:79-95,164-193` [18] | Define announced phase/error/retry states; preserve input and explain recovery. |
| Success/session | Reader appears after `list_docs`; status bar says paired; tree is immutable.[1] | `wasm/reader.html:589-595` [17]; `wasm/src/lib.rs:244-270` [18] | Make success and “session ended” explicit without changing the protocol. |
| Content/errors | Skeleton, 25 MB card, generic non-200/fetch-failed cards; no managed-page doc retry.[1] | `wasm/reader.html:732-790` [17]; protocol statuses.[23] | Distinguish too-large, missing, read failure, and disconnected; give next action. |
| Mobile | Single-column drawer at ≤767px, 84vw/320px max, larger controls, internal table/code scrolling.[1] | `wasm/reader.html:446-479` [17]; W3C Reflow.[11] | Accept 320 CSS px and phone viewport; define focus/escape/return drawer behavior. |
| Keyboard/screen reader | WASM tree is click-only `div` content; pairing error/input lack semantic announcement/label. | `wasm/reader.html:804-826` [17]; W3C Keyboard and Labels.[8][9] | Keyboard-operable tree/drawer, visible focus, labels, state relationships, live status. |
| Motion | Transitions exist; no reduced-motion rule observed.[1] | `wasm/reader.html:446-458` [17]; MDN reduced-motion.[13] | Disable/reduce nonessential motion under the user preference. |
| Browser floor | Module + generated WASM client + secure WebSocket; no fallback UI.[1] | `wasm/reader.html:539-540,828-829` [17]; `wasm/Cargo.toml:8-23` [24] | Declare evergreen feature floor and show unsupported-browser recovery. |

## Unresolved questions for issue #16

1. What exact first-screen terms and copy distinguish the **sender**, the **pairing code**, the **one-time session**, and the **reader** without assuming developer knowledge?
2. Which pairing phases must be visible and announced: connecting, key exchange, loading tree, ready, timeout, wrong/expired, rendezvous unavailable, protocol mismatch, and sender/session disappearance?
3. Does retry preserve the code and focus the same field, and when should the user be told to request a fresh code rather than retry?
4. After a successful pair, what is the contract for a broken pipe: show a session-ended card, offer re-pair in place, or return to the first screen while retaining the URL and clearing the consumed code?
5. What is the accessibility acceptance test for tree navigation, directory expansion, mobile drawer focus, Escape, visible focus, error/status announcements, and HTML-document framing?
6. Does the 320 CSS-pixel/reflow target cover all required content, or are code blocks and tables explicitly allowed to scroll horizontally under the applicable WCAG exceptions?
7. Which evergreen browser versions and mobile browser engines are in the support matrix, and what exact unsupported-browser message should appear when modules, WASM, or secure WebSocket are unavailable?
8. Is direct-link `?code=` part of the managed URL contract? The current source auto-fills and then strips the query after the pairing attempt: `wasm/reader.html:831-845` [17].
9. Should document-level errors have a retry action, and should the reader expose a session-health indicator while preserving the immutable tree/protocol?

## Short conclusion

The shipped surface already has a coherent visual baseline, mobile drawer, one-time pairing implementation, document-type handling, and useful low-level error distinctions. The main consumer risk is not missing infrastructure; it is that the user is asked to infer the journey and assistive technology is not told about important state changes. #16 should decide the copy/state machine and accessibility/mobile acceptance contract first. A later implementation pass can then stay narrow: UI semantics, status announcements, focus/retry behavior, reduced-motion CSS, and unsupported-browser recovery, with no production protocol or account-model change.

## Sources

[1] https://wormhole.zahranm.cloud/reader
[2] https://github.com/SulthanZahran1/docscrying/issues/1
[3] https://github.com/SulthanZahran1/docscrying/issues/7
[4] https://github.com/SulthanZahran1/docscrying/issues/12
[5] https://github.com/SulthanZahran1/docscrying/issues/14
[6] https://github.com/SulthanZahran1/docscrying/issues/15
[7] https://github.com/SulthanZahran1/docscrying/issues/16
[8] https://www.w3.org/WAI/WCAG22/Understanding/keyboard.html
[9] https://www.w3.org/WAI/WCAG22/Understanding/labels-or-instructions.html
[10] https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html
[11] https://www.w3.org/WAI/WCAG22/Understanding/reflow.html
[12] https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html
[13] https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-reduced-motion
[14] https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Modules
[15] https://developer.mozilla.org/en-US/docs/WebAssembly
[16] https://developer.mozilla.org/en-US/docs/Web/API/WebSocket
[17] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/wasm/reader.html
[18] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/wasm/src/lib.rs
[19] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/cli/src/site.html
[20] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/cli/src/serve.rs
[21] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/cli/src/open.rs
[22] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/cli/src/index.rs
[23] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/cli/src/protocol.rs
[24] https://github.com/SulthanZahran1/docscrying/blob/503d55e52cf6348081829d1697bfc9c1ed593947/wasm/Cargo.toml
