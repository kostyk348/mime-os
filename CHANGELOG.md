# Changelog

All notable changes to MIME-OS / EMLBox are documented here. Versions follow SemVer.
The release history below is reconstructed from the git tags and their annotations
(originally written in Russian; translated here, kept terse where the tag message was
terse).

## [v0.17.0] — 2026-11-03 — all fronts: query nesting, time travel, hard gates

An engineering release: the two most-requested language features land (X-Query nesting
from the roadmap, KV time travel enabled by the append-only design), and the project's
quality gates go from advisory to blocking.

### Added
- **X-Query nesting** (`src/query.rs` v0.3) — parentheses at any depth with standard
  precedence (`NOT` > `AND` > `OR`): `(X-Tag == "a" OR X-Tag == "b") AND NOT X-Timestamp < 100`.
  Implemented as a quote-aware lexer + recursive-descent parser over an AST (`Q::Leaf /
  Not / And / Or`), so quoted values now survive keywords too: `X-Note == "buy (milk)
  AND eggs"` is one clause. Legacy `query::parse` keeps its AND-list contract; `fs index`
  validates through `query::validate` (parenthesized queries are now legal there).
- **KV time travel** (`src/kv.rs`) — the append-only delta log is a history:
  `kv::table_at(b, table, n)` / `kv::get_at(...)` reconstruct the table state after the
  first `n` deltas of the table (`--at 0` = the base section; a prefix longer than the
  log is an error, not "now"). The replay order is fixed by `(ts, writer)`, so prefixes
  are identical on every device. CLI: `emlbox kv get <path> <table> <key> --at N`.
- **KV audit history** — `kv::history(b, table, key?)` and `emlbox kv history <path>
  <table> [key]`: writer#seq, timestamp, op, key and value per delta, straight from the
  file. Filterable by key; an unknown table yields an empty log, not an error.

### Fixed
- **Windows: "user-mapped section" (os error 1224) when rewriting rev cells** — `type_mark`,
  `rewrite`, `field_mark` and the `branch` references pass held an `EmlBox` mmap view of a
  cell while rebuilding that same file; on Unix a mapped file can be rewritten, on Windows
  it cannot. All four paths now read what they need under the mapping and close it before
  the write (the listing slice is copied into a `Vec` inside the scope). The whole suite
  passes on windows-latest for the first time.

### Changed
- **CI hardened** (`.github/workflows/ci.yml`): the build job runs the full suite on a
  **three-OS matrix** (ubuntu / macos / windows); `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` are now blocking gates instead of
  `clippy || true`; a tag-push `release` job builds `emlbox` binaries for
  linux-x86_64 (musl), macos-aarch64 and windows-x86_64 and attaches them to the
  GitHub release automatically.
- **Clippy: zero warnings** — all 37 pre-existing warnings fixed: type aliases for the
  complex signatures (`rev::SiteMap`, `rev::RevDiff`, `rev::Recon`, `tagdb::Record`),
  iterator loops instead of index loops (`rev` decompiler block walkers, `diff` line
  merge), a `while let` frame loop in `sync`, collapsible `if let` in `fs`, unused
  imports in tests. `cargo fmt` normalized across the codebase.
- Crate version aligned with the release tags: 0.1.0 → 0.17.0.

### Tests
- 79 tests green (was 69): 6 new query tests (AST shape, parentheses, precedence,
  quoted values, legacy `parse` contract, error cases) and 4 new KV time-travel tests
  (set/del prefixes, RGA list prefixes, audit history, cross-writer determinism with the
  same-second LWW contract documented in-test).

## [v0.16.0] — 2026-10-30 — build hardening, docs & promo

A documentation and promotion release with two small compile fixes that make the whole
suite honest again: no behavior changes — the format and the CLI surface are exactly as
they were at v0.15.0.

### Fixed
- **The `emlbox` binary target compiles again**: E0716 (dropped temporary) in the
  `rev strings` handler of `src/main.rs` — a `let`-binding, exactly what rustc suggested.
- **`tests/sync.rs` compiles and passes again**: five `Delta` initializers had fallen
  behind the struct after the RGA lists of v0.7 added `id`/`after` — updated with
  `id: None, after: None` (as `set`-op deltas are constructed in `kv.rs`).
- **CI runs the full suite now**: `cargo build --all-targets` + `cargo test` (69 tests
  green), not just `cargo build --lib`.
- The v0.15.0-era build state is documented in [LIMITATIONS.md](LIMITATIONS.md) →
  *Repository state* — with what was fixed and what has not changed.

### Added
- **`README.md` rewritten in English** in the CapsuleCraft house style: short tagline,
  bold intro, language-switch line, emoji feature bullets, the full layer/command table,
  condensed practical scenarios, container format + invariants, the measured benchmark
  tables (numbers unchanged), the X-Query reference, the cell-level reverse engineering
  section, a releases table, honest limitations, a checkbox roadmap, contributing,
  contributors and license sections.
- **`README.ru.md`** — the Russian version with the same section order. The old
  Russian-only `README.md` was the source of truth; every real feature was carried over
  (translated, not dropped).
- **`CHANGELOG.md`** (this file) — release history reconstructed from the 28 git tags.
- **`LIMITATIONS.md`** — the honest engineering list: no Runner sandbox, LWW merge for KV
  keys, no file locking, no TLS/auth on the sync wire, the `objdump` dependency, and a
  factual "Repository state" snapshot of what builds at master.
- **`CONTRIBUTING.md`** — PR workflow, commit style, the zero-dependency policy for the
  core and the rules for changing the format spec.

### Changed
- Language split: `README.md` is now the English primary document; the Russian text moved
  to `README.ru.md` (previously `README.md` was Russian-only).

---

## Release history (from git tags)

28 tagged releases between 2026-08-12 and 2026-09-13. After v0.15.0 the only commits were
CI-workflow experiments (2026-10-16 — added and reverted twice) and README badge
polishing, so v0.15.0 remains the last code release before this docs release.

| Version | Date | What |
|---|---|---|
| v0.15.0 | 2026-09-13 | `rev decomp-all` — mass decompilation of a whole binary into one `.c`; `rev stats` (functions / edges / size top). FTL: 288 functions of ShipManager → 33k lines |
| v0.14.2 | 2026-08-23 | GUI: Rev tab — decompile a rev cell with a button (with struct fields) |
| v0.14.1 | 2026-08-23 | Struct field names: `rev field <dir> <func> 0x670 scrap`; decomp substitutes `[arg0+scrap]` |
| v0.14.0 | 2026-08-23 | Decompiler readability: parameters in the body (`rbx=rdi` → `arg0`, `esi` → `arg1`), clobber tracking, `std::string` suppression, prologue cleanup |
| v0.13.1 | 2026-08-23 | `rev report` — whole-binary summary (functions, vftables, strings, entropy, top callees, decomp sample) |
| v0.13.0 | 2026-08-23 | Decompiler v0.4 — local variables (`local_8`/`param_1`, Ghidra style); validated on libc `strlen` (real if/else/while from glibc) |
| v0.12.3 | 2026-08-23 | Decompiler: while loops (backward jcc, goto-entry suppression), call arguments from `mov`, prologue cleanup |
| v0.12.2 | 2026-08-22 | Decompiler: conditions from `cmp`/`test` (`if (eax > 0)`), signatures with types from the wave (`// f(Player* arg0)`) |
| v0.12.1 | 2026-08-22 | CFG structuring in the decompiler — blocks by labels and conditional branches, if/else |
| v0.12.0 | 2026-08-22 | Decompiler (asm → pseudo-C lifter: mov/lea/arithmetic/calls/jumps), `rev strings`, `rev dot` (Graphviz) |
| v0.11.1 | 2026-08-22 | `rev vftables` — automatic vtable search (runs of pointers in `.text`). Demos: C++ shapes, libc `_IO_jump_t` |
| v0.11.0 | 2026-08-22 | `rev` on real binaries (libc: 2354 functions in 0.5 s) + `rev recon` (strings / entropy regions before objdump) |
| v0.10.3 | 2026-08-22 | `setup-home` — home mesh in one command (bookmarks + notes containers, daemon autostart via Hyprland `exec-once`, config backup) |
| v0.10.2 | 2026-08-22 | `mail watch` — background mail reception. Full mobile loop: phone → email → Maildir → watch → database → mesh → all devices |
| v0.10.1 | 2026-08-22 | GUI: Doc tab (view document, add/revert buttons) + app-sync demo in a live mesh |
| v0.10.0 | 2026-08-19 | `sync daemon` — live mesh: nodes sync automatically on a timer (serve + periodic converge) |
| v0.9.1 | 2026-08-18 | `doc log` (change history), `doc revert` (roll back deltas), `sync converge` (auto-convergence from all peers) |
| v0.9.0 | 2026-08-18 | `doc edit` — collaborative editor (edit in `$EDITOR`, LCS diff → add/set/del deltas automatically) |
| v0.8.1 | 2026-08-18 | Full CRDT document set — `list-set`/`list-del` (replace/delete by id), `doc set`/`doc del` |
| v0.8.0 | 2026-08-17 | Collaborative documents on .eml — `doc init/add/list` (RGA lines, lossless sync) |
| v0.7.1 | 2026-08-17 | X-Query OR/NOT, fsync mode (`EMLBOX_NO_FSYNC`), 3-node mesh demo |
| v0.7.0 | 2026-08-17 | `repair` (recovery after tear-write), RGA lists (`kv add/list`, conflict-free convergence) |
| v0.6.0 | 2026-08-17 | Compaction (deltas → base), ed25519 signatures (`EMLBOX_SEED`), rev hypothesis branches (In-Reply-To, wave isolation, rollback), `rev-mcp` in opencode config |
| v0.5.0 | 2026-08-16 | X-Encoding (deflate / aes-256-gcm for sections + deltas), SMTP bridge (`mail pack/apply/receive`), site generator (posts = containers), GUI `emlbox-gui` (egui) |
| v0.4.0 | 2026-08-16 | rev wave with call-site dataflow (prologue slots, rip normalization), version diff with body normalization, MCP server `rev-mcp` (8 tools for LLMs) |
| v0.3.0 | 2026-08-16 | Cellular reverse engineering without Ghidra — objdump → .eml graph of functions, type waves over `References`, semantic clusters, digests for diffing. rev CLI + 4 tests |
| v0.2.0 | 2026-08-16 | Network delta-sync — per-writer chains (`X-Writer-ID`/`X-Delta-Seq`/`X-Prev-Hash`), verbatim merge, LWW replay, per-chain verify, `sync export/push/pull/apply/heads`, `kv set --writer`, idempotent delivery |
| v0.1.0 | 2026-08-12 | First release: EMLBox — single-file .eml container OS stack (mmap reader, two-level index, append-only delta log, KV, tagdb, pack/unpack, Apache-2.0) |
