# Contributing to MIME-OS / EMLBox

Thanks for taking the time to help. The project is small and opinionated on purpose —
please keep changes focused. Both English and Russian are fine here: the repo's history
mixes them, and so do commit messages.

## Ground rules

1. **The `.eml` format is the contract.** A container must stay a valid RFC-822 / MIME
   document that ordinary mail tooling can read. Never introduce a side channel that only
   EMLBox can parse.
2. **Zero dependencies for the core.** The Cargo dependency list is deliberately tiny
   (`serde`/`serde_json`, `sha2`, `memmap2`, `flate2`, `aes-gcm`, `rand`, `ed25519-dalek`,
   plus the optional `eframe` behind the `gui` feature). A new dependency needs strong
   justification in the PR description.
3. **Append-only stays append-only.** An append must never rewrite the base prefix —
   existing byte offsets are eternal (see the invariants in [`docs/FORMAT.md`](docs/FORMAT.md)).
4. **Honesty over marketing.** If a feature has a gap, document it (README +
   [LIMITATIONS.md](LIMITATIONS.md)) instead of hiding it.

## Workflow

1. Fork or create a feature branch (`feat/...`, `fix/...`, `docs/...`).
2. Make the change, with tests.
3. Open a pull request; CI must be green.
4. Merge after review. Notable changes get a line in [CHANGELOG.md](CHANGELOG.md).

## Before you open a PR

```bash
cargo build --lib     # what CI runs today
cargo test --lib      # library unit tests
cargo test            # full suite — see the note below
```

CI currently builds the library target only. A bare `cargo test` may stop at the compile
step because of known master-state issues (binary target, some integration tests) — see
[LIMITATIONS.md](LIMITATIONS.md) → *Repository state*. Make sure the targets your change
touches build and pass; do not rely on `cargo build --release` alone.

## Commit style

- Conventional Commits: `feat:`, `fix:`, `docs:`, `ci:`.
- Messages may be in Russian or English — match the surrounding history.

## Changing the format spec

- Update [`docs/FORMAT.md`](docs/FORMAT.md) in the same PR, before or with the code.
- Add or adjust tests that pin the invariants (layout offsets, hash-chain, tail index,
  append behaviour). The tamper/tear-write tests are the last line of defence.
- Old containers must keep mounting: the base prefix of an existing file is sacred.

## Reporting bugs

Open an issue with: OS, rustc version, the exact command you ran, what you expected and
what happened — plus the offending container if possible, with private content removed.

Security issues: please do **not** open a public issue — contact the maintainer directly.

## License

By contributing you agree that your contributions are licensed under Apache-2.0
(see [`LICENSE`](LICENSE)).
