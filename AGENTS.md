# Coding Agent Guidelines for slack

Rust Slack CLI with OAuth (PKCE) authentication, multi-profile management,
and generic + wrapper Slack Web API access. Fork of
[tumf/slack-rs](https://github.com/tumf/slack-rs) (MIT), renamed to `slack`.

## Fork Policy

- `origin` = GitHub `averagechris/slack` (canonical); GitHub `tumf/slack-rs`
  is retained as the read-only `upstream` remote. Historical SourceHut releases
  remain preserved.
- Review upstream changes (supply-chain focus) before porting anything.
  Never blind-merge upstream.
- **Upstream reviewed through e6a7ba03 (v0.1.71) on 2026-07-02.** Future
  upstream review should start after that commit.
- Requirements, decisions, and the living backlog are in `docs/roadmap.md`.
  The audit that motivated the fork is `docs/fork-audit.md` (do not edit it).

## Build & Test Commands

`cargo` is provided via nix until `flake.nix` lands:

```bash
nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt --command cargo <args>
```

### Building
```bash
cargo build                    # Build debug version (binary: slack)
cargo build --release          # Build optimized release version
cargo check                    # Fast compile check without codegen
```

### Testing
```bash
cargo test                     # Run all tests
cargo test <test_name>         # Run single test by name
cargo test -- --nocapture      # Show println! output during tests
cargo test --lib               # Run only library tests
cargo test --test <file>       # Run specific integration test file
```

**Examples:**
```bash
cargo test test_api_call_with_form_data  # Run single test
cargo test oauth                          # Run all tests matching "oauth"
cargo test --test api_integration_tests   # Run tests/api_integration_tests.rs
```

### Linting & Formatting
```bash
cargo fmt                      # Format all code
cargo fmt -- --check           # Check formatting without modifying
cargo clippy --all-targets -- -D warnings   # Lint, fail on warnings (CI standard)
```

Run `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
before landing any change.

## Release flow

- The routine release interface is exactly these two commands; Tiny should run
  the readiness check first and proceed only when it succeeds:

    nix run .#release -- --version X.Y.Z --check
    nix run .#release -- --version X.Y.Z

  The non-mutating check fails fast on a dirty, stale, or diverged checkout;
  a missing or unexpected GitHub origin; invalid or downgrade versions; and
  local or remote tag conflicts. The release prepares the versioned tree,
  validates that prepared tree with the preset fmt, clippy, and test gates,
  then builds and verifies the artifact and checksum before atomically
  publishing `main` and its annotated tag with a lease. The read-only GitHub
  workflow builds the two supported platform artifacts; a maintainer verifies
  and publishes them manually. After publication, rerun the exact same command to resume: only an
  exact matching version, tag, and main state continues, while mismatches fail
  closed. Successful publication leaves a new empty `@` above `main`.
- `builds/release-linux-x86_64.yml` and the local Pages apps are archival.
  Never use them for a future release. Follow `docs/release.md` to publish the
  four verified GitHub assets and dispatch the downloads site.
- `prepare-release` and `release-tag` are lower-level recovery tools only.
  `build-pages` and `publish-pages` are preserved solely for historical
  SourceHut archaeology. Do not compose any of them into the normal path.

## Project Structure

```
src/
├── main.rs           # CLI entry point (clap-based parsing and dispatch)
├── lib.rs            # Library root with module exports (lib name: slack)
├── api/              # Slack API client, call args/envelope/guidance
├── auth/             # Auth commands (login, logout, status, export/import),
│                     #   cloudflared tunnel, manifest generation, crypto, i18n
├── cli/              # clap arg definitions (args.rs), handlers, introspection
│                     #   derived from the clap model (introspect.rs)
├── commands/         # Wrapper commands (msg, react, conv, thread, users, file,
│                     #   search, doctor, config, write guards)
├── debug.rs          # Debug logging with token redaction
├── idempotency/      # Idempotency key store for write commands
├── oauth/            # OAuth flow (PKCE, callback server, ports, scopes)
├── profile/          # Profile config + keyring token storage
└── skills/           # Embedded agent skill installer (install-skills)

config/cli.toml       # Single source of truth for the binary name
skills/slack-rs/      # Agent skill docs (embedded into the binary)
tests/                # Integration tests
docs/                 # roadmap.md (tracker), fork-audit.md (read-only), guides
```

## Storage Reality

- Tokens and OAuth client secrets live **only in the OS keyring**
  (`KeyringTokenStore`, service name `slack`, kept in sync with
  `config/cli.toml`). One entry per profile identity (`{team_id}:{user_id}`,
  a JSON map holding bot + user tokens) plus one entry per profile for the
  OAuth client secret (`oauth-client-secret:<profile>`).
- `profiles.json` holds non-secret metadata only and doubles as the profile
  index (the keyring cannot enumerate entries).
- There is no plaintext token file and no `SLACK_TOKEN` env auth. The
  one-time `slack auth migrate` imports a legacy `tokens.json` and shreds it.
- Tests must never touch the real keyring: use `InMemoryTokenStore`, or
  `slack::profile::use_mock_keyring()` (debug-only; process-global, so use
  `serial_test`). Debug builds also honor `SLACK_KEYRING_MOCK=1` for
  spawned-binary tests.

## Failure Modes

| Do not do this | Why / corrected behavior |
| --- | --- |
| Add hosted PR CI, automated release publishing, codecov, Homebrew formulae, or crates.io release plumbing | This fork is Nix-native (R2) with local PR validation. The sole GitHub workflow is a read-only, tag-triggered release artifact builder; publication remains manual. |
| Reintroduce env-var token auth (`SLACK_TOKEN`) or plaintext token storage | R1: secure by default. Encrypted export/import is the migration path between machines. |
| Reintroduce the `--client-secret` raw CLI flag | Removed per audit S6 — secrets land in shell history/process lists. Secrets come only from env var (`SLACKRS_CLIENT_SECRET` / `--client-secret-env`), file (`--client-secret-file`), or interactive prompt. |
| Reintroduce ngrok tunnel support or the `demo` command | Dropped per D4 as dead code. Cloudflared tunnel login stays. |
| Edit `~/.agents/skills/` or other distributed skill copies | Source of truth is `./skills/` in this repo. Edit `./skills/<name>/SKILL.md` and let installs propagate. |
| Hand-roll new argument parsing instead of extending the clap definitions in `src/cli/args.rs` | The CLI is clap-based (D3); introspection (`commands --json`, `schema`, `--help --json`) is derived from the clap model in `src/cli/introspect.rs` and must stay derived, never hand-mirrored. |
| Edit `docs/fork-audit.md` | Historical record of the fork audit. |

## Code Style Guidelines

### Module Organization
- Each module has a `mod.rs` with documentation and re-exports
- Start files with doc comments: `//! Module description`
- Group related functionality in submodules

### Imports
- Use crate-relative imports: `use crate::oauth::types::OAuthError;`
- Group imports: std, external crates, then crate modules
- Re-export commonly used types in `mod.rs`

### Error Handling
- Use `thiserror` for all custom errors; enums with descriptive variants
- Return `Result<T, CustomError>` from fallible functions; use `?`
- Avoid unwrap/expect in library code

### Types & Async
- Derive `Debug` for all types; `Clone` only when needed
- `serde` derives for serializable types
- `tokio` runtime: `#[tokio::main]` / `#[tokio::test]`

### Naming Conventions
- **Modules**: `snake_case`; **Types**: `PascalCase`; **Functions**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`; **Error types** end with `Error`

### Documentation
- `///` doc comments for public APIs; `//!` module-level docs
- Explain *why*, not *what*

### Testing
- Integration tests in `tests/`; use `httpmock`/`wiremock` for HTTP mocking
- Name tests descriptively: `test_api_call_with_form_data`
- Tests must not touch the network or real credential stores

## Dependencies

**Core:** `tokio` (full), `reqwest` (json, rustls-tls), `serde`/`serde_json`/`serde_yaml`
**Security:** `keyring` (OS credential stores; pure-Rust secret-service crypto), `aes-gcm`, `argon2`, `sha2`, `base64`, `rand` (encrypted export/import)
**CLI:** `rpassword`, `directories`, `arboard`, `regex`, `url`, `thiserror`
**Testing:** `tempfile`, `wiremock`, `httpmock`, `serial_test`

Keep dependencies minimal — only add when necessary, and review the
supply chain of anything new.

## Version Control

- Use `jj` (Jujutsu), not raw git. Conventional commits (D8): `feat:`,
  `fix:`, `chore:`, etc.; `!`/`BREAKING CHANGE` for majors.

## Skill Source of Truth

- The source of truth for skill definitions in this repository is `./skills/`
- Do not edit `~/.agents/skills/` or any other distributed/synced copy directly
- When a skill needs to be changed, first find and edit `./skills/<skill-name>/SKILL.md`
- Before making claims about skill behavior or operations, verify the source-side definition rather than a distributed copy
