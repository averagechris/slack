# slack

Slack CLI (Rust) — OAuth authentication, multi-profile, full Slack Web API access.

Forked from [tumf/slack-rs](https://github.com/tumf/slack-rs) (MIT) at v0.1.71.
Maintained at [github.com/averagechris/slack](https://github.com/averagechris/slack);
the former SourceHut repository and releases are historical records.

Designed following [Agentic CLI Design](https://dev.to/tumf/agentic-cli-design-7-principles-for-designing-cli-as-a-protocol-for-ai-agents-2c10) principles — structured JSON output, non-interactive operation, safe-by-default.

Key features:
- OAuth PKCE authentication with Cloudflare Tunnel support
- Multiple workspace profiles with independent credentials
- Generic API access + convenience wrapper commands
- Smart retry with exponential backoff and rate limit handling
- Encrypted profile export/import (AES-256-GCM + Argon2id)

## Installation

### Hosted downloads

Prebuilt, reproducible release tarballs (with SHA-256 checksums) are
published at <https://averagechris.srht.site/slack/>:

```bash
curl -LO https://averagechris.srht.site/slack/downloads/slack-vX.Y.Z-<platform>.tar.gz
curl -LO https://averagechris.srht.site/slack/downloads/slack-vX.Y.Z-<platform>.tar.gz.sha256
sha256sum -c slack-vX.Y.Z-<platform>.tar.gz.sha256
tar -xzf slack-vX.Y.Z-<platform>.tar.gz
install -m 0755 slack-vX.Y.Z-<platform>/slack ~/.local/bin/slack
```

### Nix

Run directly from the GitHub flake, or install into your profile:

```bash
nix run github:averagechris/slack -- --help
nix profile install github:averagechris/slack
```

### From source

```bash
git clone https://github.com/averagechris/slack.git
cd slack
nix build            # or: cargo build --release
```

See [docs/authentication.md](docs/authentication.md) for prerequisites
(Slack App credentials) and first-login setup.

## Agent Skills

Install embedded agent skill documentation for OpenCode/agent runtimes:

```bash
slack install-skills           # → ./.agents/skills/slack-rs/
slack install-skills --global  # → ~/.agents/skills/slack-rs/
```

## Quick Start

```bash
# 1. Authenticate (Cloudflare Tunnel — simplest)
slack auth login my-workspace --cloudflared

# 2. Call any Slack API method
slack api call chat.postMessage channel=C123 text="Hello!"

# 3. Use wrapper commands
slack msg post C123 "Hello!"
slack conv list
slack search "quarterly report" count=10
```

For detailed setup (manual OAuth, remote auth, credential export/import), see [docs/authentication.md](docs/authentication.md).

## Usage

### API Calls

```bash
# Generic — call any Slack Web API method
slack api call <method> [key=value...]
slack api call users.info user=U123456
slack api call conversations.history channel=C123456 limit=50

# Form-encoded arguments
slack api call chat.postMessage channel=C123 text="Hello" thread_ts=1234567.123
```

### Wrapper Commands

| Command | Description |
|---------|-------------|
| `msg post <channel> <text>` | Post a message |
| `msg update <channel> <ts> <text>` | Update a message |
| `msg delete <channel> <ts>` | Delete a message |
| `conv list` | List conversations |
| `conv history <channel>` | Get conversation history |
| `conv search <pattern>` | Search conversations by name |
| `conv select` | Interactively select a conversation |
| `search <query>` | Search messages |
| `users info <user>` | Get user info |
| `users cache-update` | Update user cache for mention resolution |
| `users resolve-mentions <text>` | Resolve user mentions in text |
| `thread get <channel> <ts>` | Get thread replies |
| `react add <channel> <ts> <emoji>` | Add reaction |
| `react remove <channel> <ts> <emoji>` | Remove reaction |
| `file upload <path>` | Upload a file |
| `file download [<file_id>]` | Download a file |
| `doctor` | Diagnostics (profile, token store, scopes) |

### Auth Commands (Quick Reference)

```bash
slack auth login [profile] --cloudflared   # Login with tunnel
slack auth status [profile]                # Check auth status
slack auth list                            # List all profiles
slack auth rename <old> <new>              # Rename profile
slack auth logout <profile>                # Remove profile
slack config oauth set/show/delete <profile>  # Manage OAuth config
```

Full auth guide: [docs/authentication.md](docs/authentication.md)

### Output Format

All commands output JSON with a unified envelope. Use `--raw` for raw Slack API response only.

```json
{
  "response": { "ok": true, "channels": [...] },
  "meta": {
    "profile_name": "default",
    "team_id": "T123ABC",
    "user_id": "U456DEF",
    "method": "conversations.list",
    "command": "conv list"
  }
}
```

```bash
slack conv list --raw | jq '.channels[].name'      # Raw mode
slack conv list | jq '.response.channels[].name'   # Default
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SLACKCLI_ALLOW_WRITE` | Control write ops (`true`/`false`) | `true` |
| `SLACK_OAUTH_BASE_URL` | Custom OAuth base URL (enterprise) | `https://slack.com` |

### Profile Storage

- `~/.config/slack-rs/profiles.json` — profile metadata (team, user, scopes)
- OS keyring (service `slack`) — access tokens + OAuth client secrets
  (macOS Keychain, Windows Credential Manager, Linux Secret Service)

Each profile stores independent OAuth config. See [docs/config-and-storage.md](docs/config-and-storage.md) for schema details.

Upgrading from a version that used a plaintext `tokens.json`? Run
`slack auth migrate` once to move tokens into the keyring (the file is
securely deleted afterwards).

## Security

- **Write protection**: Set `SLACKCLI_ALLOW_WRITE=false` to prevent accidental writes
- **Tokens**: Stored exclusively in the OS keyring, never logged; no plaintext token files or env-var token auth
- **Export/Import**: AES-256-GCM encryption with Argon2id key derivation
- **Rate limiting**: Automatic retry with exponential backoff + jitter

For full security specification, see [docs/security.md](docs/security.md).

## Development

The Nix flake is the single source of truth for building, testing, and
releasing:

```bash
nix develop            # dev shell (cargo, clippy, rust-analyzer, jj, ...)
nix flake check        # build + fmt checks + release artifact
nix run .#ci-fmt       # rustfmt + alejandra format check
nix run .#ci-clippy    # clippy, warnings denied
nix run .#ci-test      # cargo test --locked
nix run .#ci-msrv      # single-threaded cargo test --locked on Rust 1.88 (MSRV)
```

There is no hosted PR CI; validation is local via the commands above.
Future releases are prepared by the SHA-pinned Fleet GitHub backend:

```console
nix run .#release -- --version X.Y.Z --check
nix run .#release -- --version X.Y.Z
```

The command atomically publishes `main` and an annotated tag. A read-only
GitHub workflow builds macOS arm64 and Linux x86_64 artifacts; maintainers
verify and publish the four release assets manually. Historical SourceHut
downloads remain available. See [docs/release.md](docs/release.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## Roadmap

See [docs/roadmap.md](docs/roadmap.md) for the fork's requirements, decisions,
and backlog.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE); see
[LICENSE](LICENSE). Upstream code (tumf/slack-rs) is MIT.

## Acknowledgments

Forked from [tumf/slack-rs](https://github.com/tumf/slack-rs). Built with [Rust](https://www.rust-lang.org/), [reqwest](https://github.com/seanmonstar/reqwest), OAuth inspired by [oauth2-rs](https://github.com/ramosbugs/oauth2-rs).

---

**Note**: Unofficial tool, not affiliated with Slack Technologies, Inc.
