# Contributing to slack

Thanks for your interest. This is a small personal/team fork of
[tumf/slack-rs](https://github.com/tumf/slack-rs), hosted at
[git.sr.ht/~averagechris/slack](https://git.sr.ht/~averagechris/slack).
Detailed coding guidelines live in [AGENTS.md](AGENTS.md); the roadmap and
backlog live in [docs/roadmap.md](docs/roadmap.md).

## Development Setup

- Rust 1.70+ — until `flake.nix` lands, run cargo via nix:

  ```bash
  nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt --command cargo <args>
  ```

- Version control uses [jj (Jujutsu)](https://jj-vcs.github.io/), not raw git.

```bash
git clone https://git.sr.ht/~averagechris/slack
cd slack
cargo build            # binary: target/debug/slack
```

## Checks Before Landing a Change

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

All three must pass. New features should include tests. Tests must not touch
the network or real credential stores (use `wiremock`/`httpmock` and temp
dirs).

## Project Structure

```
src/
├── main.rs           # CLI entry point and command routing
├── lib.rs            # Library root (lib name: slack)
├── api/              # Slack API client, call args/envelope/guidance
├── auth/             # Auth commands, cloudflared tunnel, manifest, crypto, i18n
├── cli/              # CLI helpers, handlers, help text, introspection registry
├── commands/         # Wrapper commands (msg, react, conv, thread, users, file,
│                     #   search, doctor, config, write guards)
├── debug.rs          # Debug logging with token redaction
├── idempotency/      # Idempotency key store for write commands
├── oauth/            # OAuth flow (PKCE, callback server, ports, scopes)
├── profile/          # Profile config + token storage (file-based today)
└── skills/           # Embedded agent skill installer

config/cli.toml       # Single source of truth for the binary name
skills/slack-rs/      # Agent skill docs (embedded into the binary)
tests/                # Integration tests
docs/                 # roadmap.md, fork-audit.md (read-only), guides
```

## Commit Guidelines

Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
`chore:`; `!` / `BREAKING CHANGE` for breaking changes. Semver is derived
from commit types.

## Code Style

See [AGENTS.md](AGENTS.md) for module organization, imports, error handling
(`thiserror`), naming, and documentation conventions. Default `rustfmt`
settings apply.

## License

By contributing, you agree that your contributions are dual-licensed under
MIT OR Apache-2.0 (see [LICENSE](LICENSE)).
