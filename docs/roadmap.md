# slack — Fork Requirements, Decisions, Backlog

Fork of [tumf/slack-rs](https://github.com/tumf/slack-rs) (MIT, forked at
v0.1.71 / `e6a7ba03`). Personal/team Slack CLI: packaged with Nix, hosted on
SourceHut (`~averagechris/slack`), managed with jj. Audit that motivated this
plan: `docs/fork-audit.md`. Reference implementation for repo mechanics:
`~/projects/linear-cli`.

This file is the living tracker: requirements and decisions are stable;
the backlog checklist below reflects what is done and what is left.

## Requirements

**R1 — Secure by default.** Tokens and OAuth client secrets live in the OS
keyring only. No plaintext token files, no env-var token auth. Secrets never
appear in logs, argv, or shell history.

**R2 — Nix-native.** `flake.nix` is the single source of truth for building,
dev shell, CI scripts, and release tooling. `nix run sourcehut:~averagechris/slack`
works for the team. No GitHub Actions.

**R3 — Agent-friendly.** Stable JSON output envelope, machine-readable
introspection (`commands --json` / `schema`), agent skills installable via
`install-skills`, exit-code contract preserved.

**R4 — Team-distribution.** Reproducible release tarballs + checksums on
SourceHut Pages, versioned jj tags, conventional-commit-driven changelog.

**R5 — Maintainable.** clap-based CLI (no hand-rolled parsing, no manually
mirrored command registry). Test suite stays green (`nix flake check`,
`ci-clippy`, `ci-test`) after every landed change.

**R6 — Upstream-aware.** GitHub upstream retained as `upstream` remote;
changes reviewed (supply-chain focus) before porting; review watermark
recorded in AGENTS.md.

## Decisions

| # | Decision | Notes |
|---|----------|-------|
| D1 | **Name: `slack`** (package and binary) | Single source of truth in `config/cli.toml`, linear-cli style. If the name collides with Slack's official CLI later, handle it in packaging (alias/symlink) |
| D2 | **Keyring-only token storage** | Remove `FileTokenStore` persistence and `SLACK_TOKEN` env auth. Encrypted export/import remains the migration path between machines. Env-auth tests are refactored or dropped; follow with a coverage pass |
| D3 | **clap migration before feature work** | Deletes hand-rolled parsing and `introspection.rs`; introspection commands rebuilt on top of clap's command model |
| D4 | **Keep:** encrypted export/import, agent skills system, introspection, cloudflared tunnel login, idempotency store. **Drop:** ngrok (dead code), `demo` command, dead demo functions, `agent-skills-rs` dep | Tunnel login keeps S4 mitigations and gets the state-mismatch-abort fix |
| D5 | **Full linear-cli release pipeline** | Flake apps (`ci-*`, `release`, `prepare-release`, `release-tag`, `build-pages`, `publish-pages`, `fetch-upstream`), SourceHut Pages downloads site, `.builds/release-linux-x86_64.yml` with build-scoped OAuth grant, reproducible tarballs |
| D6 | **Dual license MIT OR Apache-2.0** | Upstream code stays MIT; new contributions dual-licensed. `LICENSE` pointer + `LICENSE-MIT` + `LICENSE-APACHE` |
| D7 | **No hosted PR CI** | Local validation via `nix flake check` + flake apps, gated by `.jj-lint.toml`. SourceHut builds for releases only |
| D8 | Conventional commits; semver via commit types; auto-generated CHANGELOG | `!`/`BREAKING CHANGE` → major, `feat:` → minor, else patch |
| D9 | **Hosting: `git.sr.ht/~averagechris/slack`** as `origin`; GitHub upstream as `upstream` | Pages site at `https://averagechris.srht.site/slack/` |
| D10 | Work is tracked as a flat backlog in this file, not phases | Keep checked items for the record |

## Security remediations (from audit)

| Audit ID | Fix |
|----------|-----|
| S1 | Keyring-only storage (D2); scrub false docs claims |
| S2 | Moot once file storage is gone; export/import files created with `OpenOptions::mode(0o600)` atomically |
| S3 | Feed stored `KdfParams` into `Argon2::new(...)`; version-gate old exports |
| S4/S5 | Callback server: ignore wrong-state requests instead of aborting; constant-time state compare; fix URL decoding; stop printing expected state |
| S6 | Remove `--client-secret` raw flag (env/file/prompt only) |
| S7 | 0600 for `profiles.json` too |
| Q7 | Port 429 retry/backoff into `call_method` so wrapper commands retry |

## Backlog

### Repo bootstrap
- [x] Rename package/binary to `slack`; add `config/cli.toml`
- [x] Dual-license: `LICENSE` pointer, `LICENSE-MIT`, `LICENSE-APACHE`, Cargo.toml
- [x] Delete cruft: `.review-gauntlet/`, `.serena/`, `.wt/`, `openspec/`,
      `.github/workflows/` contents, `Formula/`, `demo` command + dead demo fns,
      `agent-skills-rs` dep, ngrok module
- [x] Rewrite AGENTS.md: fork policy, failure modes, upstream review watermark
- [x] Fix docs drift (commands.md, CONTRIBUTING.md) or prune to accurate set
- [ ] Point `origin` at `git.sr.ht/~averagechris/slack`, GitHub as `upstream`

### Nix packaging & dev environment
- [x] `flake.nix` (linear-cli pattern): `buildRustPackage` + `cargoLock`,
      metadata from Cargo.toml/config/cli.toml, devShell (cargo-audit, -deny,
      -machete, -nextest, rust-analyzer, alejandra, nixd, jj), alejandra formatter
- [x] Flake apps: `ci-fmt`, `ci-clippy`, `ci-test`, `fetch-upstream`
- [x] `.envrc` (`use flake`, git-ignored), `.jj-lint.toml` (clippy gate)
- [ ] `nix flake check` green

### Security hardening
- [x] Keyring-only `TokenStore` (macOS Keychain + Linux secret-service);
      delete file persistence + `SLACK_TOKEN` auth; one-time migration
      command from `tokens.json` (then shred it)
- [x] Refactor/drop `SLACK_TOKEN` env tests; keep in-memory store for tests
- [ ] S3: use stored KDF params in `derive_key`
- [ ] S4/S5: callback server fixes (ignore wrong state, constant-time compare,
      URL decode, no state in errors)
- [x] S6: remove `--client-secret` raw flag
- [ ] S7: 0600 `profiles.json`; atomic 0600 creation for export/import files

### clap migration
- [ ] Rebuild CLI on clap derive; preserve command surface, flags, exit codes
- [ ] Rebuild `commands --json` / `schema` / `--help --json` from clap's model
      (delete `introspection.rs`)
- [ ] Delete hand-rolled parsing in `main.rs` / `cli/mod.rs` / `handlers.rs`
- [ ] Shell completions (clap_complete)
- [ ] Integration tests assert CLI surface parity before/after

### Reliability & coverage
- [ ] Q7: 429 retry/backoff in `call_method` (wrapper commands)
- [ ] Coverage pass: OAuth callback server happy path, handler/dispatch paths,
      login orchestration; shore up gaps opened by keyring/env-auth removal

### Release pipeline
- [ ] `release-artifact` reproducible tarballs (+ sha256, manifest.json)
- [ ] `prepare-release`, `release-tag`, `build-pages`, `publish-pages`,
      `release` orchestrator flake apps
- [ ] `.builds/release-linux-x86_64.yml`, SourceHut Pages site
- [ ] `CHANGELOG.md` seeded; first tagged release `v0.2.0`

### Feature backlog (post-bootstrap, as team needs emerge)
- [ ] `--blocks` (Block Kit) on `msg post`
- [ ] `conversations.open` wrapper (DM users directly)
- [ ] `users lookup --email`
- [ ] Pagination flags (`--cursor` / `--all`) on history/replies/search
- [ ] pins / bookmarks / channel management wrappers

### Continuous
- Upstream review cadence via `fetch-upstream` + merge-review skill;
  watermark in AGENTS.md
- Tests + clippy green after every landed change; conventional commits
