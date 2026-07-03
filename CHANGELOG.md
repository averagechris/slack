# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Entries are generated from conventional-commit summaries by the
`prepare-release` flake app; this seed entry was written by hand.

This project is a fork of [tumf/slack-rs](https://github.com/tumf/slack-rs)
(MIT), forked at upstream v0.1.71 (`e6a7ba03`). The first tagged release of
the fork will be v0.2.0.

## Unreleased


## v0.3.0 - 2026-07-03

### Added

- `msg post --blocks <json|@file>` sends Block Kit blocks with
  `chat.postMessage` (validated as a JSON array; positional text becomes
  optional fallback text).
- `msg post --user <user_id>` posts to a DM by opening it first via
  `conversations.open` (mutually exclusive with the positional channel).
- `conv open <user_id>...` wrapper for `conversations.open` (DM or group
  DM); idempotent and not gated by `SLACKCLI_ALLOW_WRITE`.
- `users lookup --email <email>` wrapper for `users.lookupByEmail` with
  friendly `users_not_found` guidance.
- Pagination flags: `--cursor` / `--all` / `--max-pages` (default 10) on
  `conv history` and `thread get`, and `--all` / `--max-pages` on `search`
  (page-based, merging matches). Envelope `meta.pagination` reports
  `pages_fetched` plus `next_cursor` / `next_page` when truncated; 429s are
  retried automatically by the client.

### Changed

- `thread get` now fetches a single page by default; pass `--all` to follow
  `next_cursor` (previously all pages were always fetched).

### Fixed

- The `release` flake app now describes the release commit
  (`chore: release vX.Y.Z`) before tagging when the working-copy commit has
  no description, so `release-tag` no longer tags undescribed commits.

## v0.2.0 - 2026-07-03

### Added

- Nix flake with reproducible packaging, dev shell, CI apps (`ci-fmt`,
  `ci-clippy`, `ci-test`), and `fetch-upstream` for upstream review.
- Release pipeline: reproducible `release-artifact` tarballs with SHA-256
  checksums, `prepare-release` / `release-tag` / `build-pages` /
  `publish-pages` / `release` flake apps, SourceHut Pages downloads site,
  and a SourceHut build manifest for the Linux x86_64 release artifact.
- One-time `slack auth migrate` command to import a legacy plaintext
  `tokens.json` into the OS keyring and securely delete the file.
- Fork audit (`docs/fork-audit.md`) and living roadmap/backlog
  (`docs/roadmap.md`).

### Changed

- **Breaking:** renamed package and binary from `slack-rs` to `slack`;
  binary name is sourced from `config/cli.toml`.
- **Breaking:** tokens and OAuth client secrets are stored exclusively in
  the OS keyring (macOS Keychain / Linux Secret Service); file-based token
  persistence was removed.
- Dual-licensed new contributions under MIT OR Apache-2.0 (upstream code
  remains MIT); added `LICENSE` pointer, `LICENSE-MIT`, and `LICENSE-APACHE`.
- Removed inherited cruft: GitHub Actions workflows, Homebrew formula,
  ngrok tunnel support, the `demo` command and dead demo functions, the
  `agent-skills-rs` dependency, and stale spec/review directories.
- clap-based CLI migration in progress: hand-rolled argument parsing and the
  manually mirrored introspection registry are being replaced by clap's
  command model.

### Fixed

- `auth logout` now removes ALL credentials for the profile from the OS
  keyring — bot token, user token, and the OAuth client secret entry —
  instead of only the bot token.
- `doctor` now checks the correct user-token key (`{team}:{user}:user`);
  previously it always reported the user token as missing.
- 429 rate-limit retry with exponential backoff now applies to wrapper
  commands via `call_method` (Q7).

### Security

- **Breaking:** removed `SLACK_TOKEN` environment-variable authentication
  and plaintext token files; encrypted export/import is the supported
  migration path between machines (S1/S2).
- Removed the `--client-secret` raw CLI flag; secrets are accepted only via
  environment variable, file, or interactive prompt (S6).
- OAuth callback server hardening: wrong-state requests are ignored instead
  of aborting the flow, state comparison is constant-time, URL decoding was
  fixed, and the expected state is no longer printed (S4/S5).
- Export/import files and `profiles.json` are created atomically with
  `0600` permissions (S2/S7).
- Encrypted export key derivation feeds stored KDF parameters into Argon2
  (S3).
