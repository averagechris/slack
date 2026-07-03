---
name: slack-rs
description: |
  Slack Web API automation via the `slack` CLI (fork of slack-rs) (Rust). Use when you need to authenticate to Slack via OAuth (PKCE), manage multiple workspace profiles, call arbitrary Slack Web API methods (e.g. chat.postMessage, conversations.list, users.info), and run safe scripted Slack operations from the terminal. Includes tunnel-assisted remote login (see auth login --help), encrypted profile export/import, and a write-safety guard via SLACKCLI_ALLOW_WRITE. Credentials are stored in file-based storage under ~/.config/slack-rs/.
---

# slack-rs - Slack Web API CLI (Rust)

Use `slack` to interact with Slack workspaces using your own OAuth credentials. It supports multiple profiles (workspaces/apps), stores credentials in file-based storage under `~/.config/slack-rs/`, and can call any Slack Web API method.

## Setup

For install, OAuth app creation, and first-time authentication, see `slack-rs/references/setup.md`.

## Make API Calls

Use generic API calls for anything supported by Slack Web API:

```bash
slack api call users.info user=U123456
slack api call conversations.list limit=200
slack api call conversations.history channel=C123456 limit=50
slack api call chat.postMessage channel=C123456 text="Hello from slack-rs"
```

### Unified Output Envelope

By default, commands output a unified structure:

```json
{
  "meta": {
    "profile_name": "default",
    "method": "conversations.list",
    "command": "api call",
    "token_type": "user"
  },
  "response": {
    "ok": true,
    "channels": []
  }
}
```

To get the raw Slack Web API response (without the envelope), use `--raw`:

```bash
slack api call conversations.list --raw
```

### Choose Bot vs User Token

If your Slack app has both a bot token and a user token, set the default token type per profile:

```bash
slack config set my-workspace --token-type user
slack config set my-workspace --token-type bot
```

Confirm with:

```bash
slack auth status my-workspace
```

For more copy/pasteable recipes, see `slack-rs/references/recipes.md`.

## Fixed Rules for Slack Posting

When posting or updating Slack messages (`chat.postMessage`, `chat.update`), follow these rules to avoid broken newlines such as literal `\\n` appearing in the final message.

1. Do not write long message bodies directly in the shell
   - Avoid relying on shell quoting or escaped `\n`
   - Generate the body with `python3 - <<'PY'` using triple-quoted strings, then assign it to a variable
2. Always verify the rendered message after posting
   - Do not treat the `chat.postMessage` / `chat.update` response alone as success
   - Re-fetch the message with `conversations.history` or `conversations.replies` and inspect the actual rendered text
3. Do not report success until verification is complete
   - This is an operational rule

Recommended flow:

```bash
TEXT="$({ python3 - <<'PY'
text = """line 1
line 2
line 3"""
print(text, end="")
PY
} )"

slack api call chat.postMessage channel=C123456 text="$TEXT"

slack api call conversations.history channel=C123456 limit=1
# or, for a threaded reply
slack api call conversations.replies channel=C123456 ts=<thread_ts>
```

During verification, inspect the fetched `text` as-is and confirm that no unintended literal `\\n` or `\\n\\n` sequences appear before treating the operation as successful.

## Introspection (Commands / Help / Schemas)

Use these commands to discover what the CLI can do and how to call it (machine-readable):

```bash
slack commands --json

slack conv list --help --json
slack msg post --help --json

slack schema --command msg.post --output json-schema
slack schema --command conv.list --output json-schema
slack schema --command api.call --output json-schema
```

## Conversation Helpers

Use the convenience commands instead of `api call` for common tasks:

```bash
slack conv list
slack conv search <pattern>
slack conv history <channel_id>
slack thread get <channel_id> <thread_ts>
```

Notes:

- Command names accept both dot and space formats (e.g. `conv.list` == `conv list`, `msg.post` == `msg post`).
- `schema` describes the default enveloped JSON output; it does not describe `--raw` output.
- `meta` is a baseline envelope and not exhaustive; additional fields may be added over time.
- `conv list` supports `--filter`, `--format`, and `--sort` (see `slack conv list --help`).
- `conv select` and `conv history --interactive` require an interactive terminal (TTY).

Example output (`slack schema --command msg.post --output json-schema`):

```json
{
  "schemaVersion": 1,
  "type": "schema",
  "ok": true,
  "command": "msg.post",
  "schema": {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
      "schemaVersion": {
        "type": "integer",
        "description": "Schema version number"
      },
      "type": {
        "type": "string",
        "description": "Response type identifier"
      },
      "ok": {
        "type": "boolean",
        "description": "Indicates if the operation was successful"
      },
      "response": {
        "type": "object",
        "description": "Slack API response data"
      },
      "meta": {
        "type": "object",
        "description": "Metadata about the request and profile",
        "properties": {
          "profile": {"type": "string"},
          "team_id": {"type": "string"},
          "user_id": {"type": "string"},
          "method": {"type": "string"},
          "command": {"type": "string"}
        }
      }
    },
    "required": ["schemaVersion", "type", "ok"]
  }
}
```

## Safe Defaults for Write Operations

Many Slack methods are write operations (posting, updating, deleting, reactions). Use the guard in environments where writes are risky:

```bash
export SLACKCLI_ALLOW_WRITE=false
```

Re-enable explicitly when you intend to write:

```bash
export SLACKCLI_ALLOW_WRITE=true
```

## Profile Backup / Migration

Export/import profiles using encrypted files (treat as secrets):

```bash
# Prompt for passphrase (recommended)
slack auth export --all --out all-profiles.enc --passphrase-prompt --yes
slack auth import --all --in all-profiles.enc --passphrase-prompt
```

For non-interactive automation options, refer to `slack auth export --help` and `slack auth import --help`.

## Configuration

Common environment variables:

- `SLACKCLI_ALLOW_WRITE`: allow/deny write operations (default: allowed)
- `SLACK_OAUTH_BASE_URL`: custom OAuth base URL (testing/enterprise Slack)

For export/import passphrase options, use `--passphrase-prompt` or see `slack auth export --help`.

## Troubleshooting

- Remote environments: use a tunnel (cloudflared) and set your profile redirect URI accordingly.

### Private channels are missing

Private channels typically require a user token. Ensure:

1. `slack config set <profile> --token-type user`
2. Your Slack app has user scopes (`groups:read`, `groups:history` / `conversations:read`, etc.)

## Useful Commands

Profile management:

```bash
slack auth list
slack auth status <profile>
slack auth rename <old> <new>
slack auth logout <profile>
```

OAuth config management:

```bash
slack config oauth show <profile>
slack config oauth delete <profile>
```
