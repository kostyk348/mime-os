# Launch posts for MIME-OS / EMLBox v0.16 (EN)

## Hacker News (Show HN)

**Show HN: EMLBox v0.16 — everything is an .eml container (Rust, zero-dependency core, full suite green again)**

EMLBox is an operating environment where the atomic unit is an RFC-822/MIME document, not
an .exe or a closed database. One `.eml` file is a whole application: view (html), logic
(python), state (KV), binary assets — plus an append-only delta log with a sha256
hash-chain, read via mmap byte-offsets (envelope → head index → tail index; mount cost is
independent of file size).

What's in the box: KV tables with delta replay, an IPC bus made of .eml messages, a logic
runner, a flat tag-DB (header-only scan, corruption-proof, torn-write safe via tmp+rename),
EML-FS where folders are queries, multi-writer delta-sync (per-writer chains, LWW merge,
TCP transport in pure std::net), ed25519-signed delta blocks, deflate/aes-256-gcm section
encoding, an SMTP bridge, CRDT documents (RGA lists), and a cell-level binary reverse
engineering tool that emits .eml function graphs with an MCP server for LLMs.

v0.16 fixed the build (the binary target had an E0716; `tests/sync.rs` had fallen behind
the `Delta` struct) — the full suite is green again (69 tests), CI runs everything, and the
docs were rebuilt bilingual (EN+RU) with an honest LIMITATIONS list. The on-disk format is
unchanged since v0.1: containers from day one still open.

https://github.com/kostyk348/mime-os

## Reddit r/rust (текст поста)

EMLBox: single-file .eml containers as an application/OS format. The part I'm most proud
of: stable byte-offsets (appends never touch the base prefix), zero-copy section reads
(6.2 ns/fetch measured), and hash-chained append-only deltas that `verify` can check after
any tampering. v0.16 is a hardening release: binary + integration tests compile again,
69 tests green, bilingual docs. Would love feedback on the container format (FORMAT.md).

## Post for X/Twitter

EMLBox v0.16: the single-file .eml container OS, rebuilt docs (EN+RU),
compile fixes, full suite green again (69 tests). Everything is .eml.
https://github.com/kostyk348/mime-os
