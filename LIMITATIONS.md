# Known limitations

Honest list of what MIME-OS / EMLBox does **not** do yet. Updated as features land — if a
limitation below is fixed, it gets removed instead of being quietly forgotten.

_Last updated: 2026-10-30 (v0.16 in progress)_

## Execution & security

- **The Runner has no sandbox.** `emlbox run` executes the `logic` section of a container
  as-is — what runs is the container author's code, with the rights of your user. A
  seccomp/nsjail sandbox is a roadmap item, not a fact.
- **Containers are not encrypted as a whole.** There is no GPG-style end-to-end envelope
  encryption. Confidentiality exists only per-section via X-Encoding
  (`create --enc aes` → aes-256-gcm, key from `EMLBOX_KEY`/`EMLBOX_PASS`); integrity comes
  from the hash-chain, plus ed25519 delta signatures when `EMLBOX_SEED` is set.
- **`verify` detects tampering, it does not prevent harm.** `verify` proves the delta
  chain is intact; it says nothing about whether the logic inside the container is safe to
  run. `unpack` rejects absolute paths and `..` (path traversal), which is a guard on file
  extraction, not a general sandbox.

## State & consistency

- **KV merge is LWW** (last-writer-wins by timestamp) with no vector clocks: two devices
  writing the same key concurrently lose one of the writes by design. Only lists get a
  conflict-free structure (RGA, `kv add/list`) and documents are CRDT (`doc`).
- **Single process per file.** There is no file locking: two processes appending deltas to
  the same container concurrently is not a supported mode.
- **No WAL.** A tear-write mid-append is recovered by `repair` (tail rebuild by block
  scan), not prevented. The tail index is rewritten on every append by design; a reader
  must not assume the file is consistent in the middle of a write.
- **Compaction is a convergence point, not garbage collection in general**: after
  `compact`, chains reset; there is no automatic reindex after hand-editing a container as
  text.

## Sync & network

- **No TLS, no authentication on the wire.** The sync transport is a spool directory of
  `.eml` messages or plain TCP (`std::net`, `sync serve/connect`). Chains are verified and
  signature-checked after the fact, but the transport itself trusts the network.
- **Convergence, not isolation.** Delivery is idempotent (dedup by writer#seq) and merge
  is deterministic LWW, so replicas converge; a hostile peer can still inject deltas that
  fail `verify` — detection is per-chain, rejection of garbage is on you.

## Reverse engineering

- **`rev` depends on objdump** from GNU binutils — there is no internal disassembler and
  no Ghidra/IDA-grade analysis.
- **The decompiler is a heuristic lifter** (mov/lea/arithmetic/calls/jumps, CFG
  structuring, type waves), not a sound decompilation: the pseudo-C is an aid to reading,
  not a proof.
- **Wave typing is inference, not recovery**: only functions that actually pass their own
  parameter into a labelled argument get typed — everything else stays honestly untyped.

## Product surface

- **The GUI is behind a feature flag** (`cargo build --features gui`) and is a viewer:
  sections, deltas, KV, documents, rev decompilation, verify. Editing flows through the
  CLI.
- **The IPC bus is local** — a spool directory dispatched by `To:`; remote IPC means
  syncing containers, not calling a network broker.
- **The site generator is minimal**: posts are `.eml` containers rendered with a
  mini-markdown subset, not a themable static-site framework.
- **EML-FS "directories" are queries** (`X-Query` / `X-Contains-ID` headers): there is no
  real hierarchy, and records have no rename/move-by-path.

## Repository state (2026-10-30, after the compile fixes)

A factual snapshot of what builds out of the box after the v0.16 cycle:

- The **whole suite builds and runs again**: `cargo build --release` and a bare
  `cargo test` are green — 69 tests across library, binary and integration targets.
  The fixes themselves were tiny: a `let`-binding for an E0716 (dropped temporary) in the
  `rev strings` handler of `src/main.rs`, and `Delta{id,after}` initializers in
  `tests/sync.rs` that had fallen behind the struct.
- CI now runs the full suite (`cargo build --all-targets` + `cargo test`), not just the
  library target.
- What has **not** changed (and is listed honestly above): no Runner sandbox, LWW for KV
  keys, single-writer-per-store assumptions, minimal site generator, no transport TLS —
  see the sections above.
