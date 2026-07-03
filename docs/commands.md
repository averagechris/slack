# Command Reference

Reference for the `slack` CLI command surface. Verified against the current
hand-rolled parser (a clap migration is planned; see `docs/roadmap.md`).

## Global Flags

These flags work in any position:

| Flag | Description |
|------|-------------|
| `--profile <name>` / `--profile=<name>` | Profile to use (default: `default`; also `SLACK_PROFILE` env) |
| `--non-interactive` | Run without interactive prompts (auto-enabled when stdin is not a TTY) |
| `--debug` | Show debug information (tokens redacted) |
| `--trace` | Show verbose trace information |

Write operations are gated by the `SLACKCLI_ALLOW_WRITE` environment variable
(default: `true`), not a CLI flag. Set `SLACKCLI_ALLOW_WRITE=false` to block
writes.

Output format defaults to a unified JSON envelope `{response, meta}`. Use
`--raw` (on commands that support it) or `SLACKRS_OUTPUT=raw` for the raw
Slack API response.

## Command Structure

```
slack [--non-interactive] <COMMAND> [SUBCOMMAND] [ARGS] [OPTIONS]
```

Machine-readable introspection:

```bash
slack commands --json                             # List all commands
slack <command> --help --json                     # Per-command help as JSON
slack schema --command <cmd> --output json-schema # JSON schema for a command
```

## Commands

### `auth` — Authentication Management

```bash
slack auth login [profile] [--client-id <id>] [--bot-scopes <scopes>]
                 [--user-scopes <scopes>] [--cloudflared [path]]
slack auth status [profile]
slack auth list
slack auth rename <old> <new>
slack auth logout [profile]
slack auth export [--profile <name> | --all] --out <file>
                  (--passphrase-env <VAR> | --passphrase-prompt) --yes
slack auth import --in <file> (--passphrase-env <VAR> | --passphrase-prompt)
                  [--yes] [--force] [--dry-run] [--json]
slack auth migrate [--path <file>]
```

- `--cloudflared` uses the manifest-first tunnel login flow: a temporary
  cloudflared tunnel serves the OAuth callback, an App Manifest is generated,
  and credentials are collected after you create the Slack App.
- Scopes are comma-separated, or `all` for the comprehensive preset.
- Export/import files are encrypted (AES-256-GCM + Argon2id).
- `auth migrate` is a one-time command that imports a legacy plaintext
  `tokens.json` into the OS keyring, then securely deletes the file.

### `config` — Profile OAuth Configuration

```bash
slack config oauth set <profile> --client-id <id> --redirect-uri <uri> --scopes <scopes>
                       [--client-secret-env <VAR>] [--client-secret-file <PATH>]
slack config oauth show <profile>
slack config oauth delete <profile>
slack config set <profile> --token-type <bot|user>
```

Client secret sources, in priority order: `--client-secret-env`, the
`SLACKRS_CLIENT_SECRET` environment variable, `--client-secret-file`,
interactive prompt. There is intentionally no raw `--client-secret` flag.

### `api call` — Generic API Access

```bash
slack api call <method> [key=value...] [--json] [--get] [--raw]
```

- `<method>`: any Slack Web API method (e.g. `chat.postMessage`)
- `key=value`: request parameters (form-urlencoded by default)
- `--json`: send parameters as a JSON body
- `--get`: use GET instead of POST
- `--raw`: output raw Slack API response (no envelope)

Includes automatic 429 retry with Retry-After/backoff.

### `search` — Search Messages

```bash
slack search <query> [--count=N] [--page=N] [--sort=TYPE] [--sort_dir=DIR]
```

Requires a user token (`search:read` user scope).

### `conv` — Conversations

```bash
slack conv list [--filter=KEY:VALUE]... [--format=FORMAT] [--sort=FIELD]
                [--sort-dir=DIR] [--types=TYPES] [--limit=N] [--include-private]
slack conv search <pattern> [--select]
slack conv select
slack conv history <channel> [--limit=N] [--oldest=TS] [--latest=TS]
slack conv history --interactive [--filter=KEY:VALUE]...
```

### `thread` — Threads

```bash
slack thread get <channel> <thread_ts> [--limit=N] [--inclusive] [--raw]
                 [--token-type=bot|user]
```

### `users` — User Information

```bash
slack users info <user_id>
slack users cache-update [--force]
slack users resolve-mentions <text> [--format=FORMAT]
```

### `msg` — Message Operations (write-gated)

```bash
slack msg post <channel> <text> [--thread-ts=TS] [--reply-broadcast] [--yes]
               [--token-type=bot|user] [--idempotency-key=KEY]
slack msg update <channel> <ts> <text> [--yes] [--idempotency-key=KEY]
slack msg delete <channel> <ts> [--yes] [--idempotency-key=KEY]
```

- Requires `SLACKCLI_ALLOW_WRITE=true` (the default).
- `--yes` confirms destructive operations in non-interactive mode.
- `--idempotency-key` prevents duplicate writes on retries.

### `react` — Reactions (write-gated)

```bash
slack react add <channel> <ts> <emoji> [--yes] [--idempotency-key=KEY]
slack react remove <channel> <ts> <emoji> [--yes] [--idempotency-key=KEY]
```

### `file` — Files

```bash
slack file upload <path> [--channel=ID] [--channels=IDs] [--title=TITLE]
                  [--comment=TEXT] [--yes] [--idempotency-key=KEY]
slack file download [<file_id>] [--url=URL] [--out=PATH]
```

### `doctor` — Diagnostics

```bash
slack doctor [--profile=NAME] [--json]
```

Shows profile config path, token store backend (OS keyring), token
availability, and scope hints.

### `install-skills` — Agent Skills

```bash
slack install-skills [source] [--global] [--json]
```

Installs the embedded agent skill docs (default source: `self`; also
supports `local:<path>`).

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (invalid arguments, API error, etc.) |
| 2 | Non-interactive error (interactive input required but unavailable) |

## Output

All commands output JSON with the unified envelope:

```json
{
  "response": { "ok": true, "...": "..." },
  "meta": {
    "profile_name": "default",
    "team_id": "T123ABC",
    "user_id": "U456DEF",
    "method": "conversations.list",
    "command": "conv list"
  }
}
```

Set `SLACKRS_OUTPUT=raw` (or pass `--raw` where supported) to get the raw
Slack API response only.
