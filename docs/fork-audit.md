# Fork Audit — slack-rs (upstream v0.1.71)

Audited 2026-07-02 as a fork candidate for a personal/team Slack CLI, packaged
with Nix, hosted on SourceHut, managed with jj. Baseline: upstream `main` at
`e6a7ba03`. Build, all ~564 tests, and `cargo clippy --all-targets -- -D warnings`
pass clean.

Verdict: **fork it**. The hard parts (OAuth/PKCE, multi-profile tokens, output
envelope, write guards, test harness) are done well. Everything wrong is
fixable and enumerated below.

## 1. Security findings

| # | Sev | Finding | Location |
|---|-----|---------|----------|
| S1 | HIGH | Docs claim OS-keyring storage; reality is plaintext `~/.local/share/slack-rs/tokens.json` (0600) holding all tokens **and** OAuth client secrets. `keyring` is not a dependency; `create_token_store()` only returns `FileTokenStore`. | `src/profile/token_store.rs:346`, false claims in `src/profile/mod.rs:5`, `src/auth/commands.rs:266`, `AGENTS.md` |
| S2 | MED | TOCTOU: secret files are `fs::write` then chmod 0600 — briefly world-readable on first creation; permissions never re-verified on load. | `src/profile/token_store.rs:254-266,212-227`, `src/auth/export_import.rs:480-492` |
| S3 | MED | `derive_key` ignores the KDF params stored in the export header and always uses `Argon2::default()`. If crate defaults change, old exports become undecryptable. | `src/auth/crypto.rs:58-72` |
| S4 | MED | Cloudflared/ngrok tunnel login exposes the OAuth callback server publicly during the flow. Mitigated by PKCE + state; but a wrong-state request aborts the whole login (DoS-able). Auth code transits the tunnel provider (PKCE keeps it unexchangeable). | `src/auth/commands.rs:1133-1149`, `src/oauth/server.rs:79-93`, `src/auth/cloudflared.rs` |
| S5 | LOW | Non-constant-time state comparison; hand-rolled URL decode corrupts multibyte UTF-8; `StateMismatch` error prints the expected state. | `src/oauth/server.rs:196-221`, `src/oauth/types.rs:23-24` |
| S6 | LOW | `--client-secret` accepted as a raw CLI flag (labeled unsafe, gated behind `--yes`) — lands in shell history. | `src/main.rs:767` |
| S7 | LOW | `profiles.json` written with default permissions (holds client_id, team/user IDs — no tokens). No key/passphrase zeroization. | `src/profile/storage.rs:133-142` |

Security strengths worth preserving: correct S256 PKCE with CSPRNG verifier and
state, callback bound to `127.0.0.1` only, strict port validation, tokens sent
only via `Authorization: Bearer`, exemplary debug redaction (`src/debug.rs`
never logs token bytes; `xox*` strings redacted from logged JSON), Argon2id +
AES-256-GCM export with fresh salt/nonce, passphrases never accepted as CLI
args, no hardcoded secrets, no panic paths from untrusted input.

## 2. Code quality findings

| # | Finding | Location |
|---|---------|----------|
| Q1 | Hand-rolled arg parsing (no clap): per-command `while i < args.len()` loops; ~4k lines across the CLI layer. | `src/main.rs` (1183 ln), `src/cli/mod.rs` (2705 ln), `src/cli/handlers.rs` (1547 ln) |
| Q2 | `src/cli/introspection.rs` (1581 ln) is a manually mirrored command registry for `--help --json` / `schema` — nothing ties it to the actual parsers; guaranteed drift. Would be derived for free under clap. | `src/cli/introspection.rs` |
| Q3 | Dead code: ~230 lines of never-called demo functions; stub `demo` command. | `src/main.rs:949-1183,182` |
| Q4 | Unused dependency `agent-skills-rs = "0.2.0"` — zero usages. | `Cargo.toml` |
| Q5 | AI-workflow sediment committed to the repo: `.review-gauntlet/`, `.serena/`, `.wt/`, `openspec/` (79 archived change folders). | repo root |
| Q6 | Docs drift: `docs/commands.md` documents nonexistent flags (`--no-color`, global `--format`, `--allow-write`); omits `thread`, `file`, `doctor`, introspection commands. `CONTRIBUTING.md` tree stale. | `docs/commands.md`, `CONTRIBUTING.md` |
| Q7 | Retry inconsistency: only generic `ApiClient::call` (used by `api call`) retries 429 with Retry-After/backoff; `call_method` (used by ALL wrapper commands) has zero retry logic, contradicting README claims. | `src/api/client.rs:209-318` vs `call_method` |
| Q8 | Triplicated help text (`print_help`, `print_usage`, per-command usage fns). | `src/main.rs`, `src/cli/help.rs` |

## 3. Feature inventory

Implemented: `auth` (OAuth login incl. manifest-first cloudflared flow,
multi-profile, encrypted export/import), generic `api call` (any method, GET/
form/JSON, retry), `msg post/update/delete` (idempotency keys), `thread get`,
`react add/remove`, `conv list/search/select/history`, `file upload/download`,
`search`, `users info/cache-update/resolve-mentions`, `doctor`, `config`,
introspection (`commands --json`, `schema`), `install-skills`.

Notable absences (all reachable via raw `api call`, no ergonomics):

- Block Kit in `msg post` (text only)
- `conversations.open` (DM a user without manual channel resolution)
- Message scheduling (`chat.scheduleMessage`)
- Pins, bookmarks, channel create/archive/invite/join/topic
- `users.lookupByEmail`, users listing
- Socket Mode / events / streaming
- Token rotation/refresh
- User-facing pagination flags (`--cursor` / `--all`) on history/replies/search

## 4. Test coverage

~434 unit + 130 integration tests; meaningful quality (proper wiremock/httpmock
mocks of real Slack endpoints, error paths, migration edge cases). Strong:
token store (32), export/import crypto (22 integration + 11 unit), conv
formatting, parsing helpers. Gaps:

- `src/main.rs` routing: zero tests
- `src/oauth/server.rs` happy path (callback receipt): untested
- Handler bodies in `src/cli/`: only leaf helpers unit-tested
- Login flow orchestration (tunnels, browser-open): untested
- Upstream CI coverage (tarpaulin/codecov) was decorative: main-only,
  `continue-on-error`, thresholds off

## 5. Disposition

Tracked decisions and the remediation roadmap live in `docs/roadmap.md`.
Upstream provenance: https://github.com/tumf/slack-rs (MIT).
