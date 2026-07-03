# Config and Storage Specification

## Overview
This document defines the exact schema and storage mechanisms for profiles and tokens.

## Config File Location
- Non-secret configuration stored in: `~/.config/slack-rs/`
- Platform-independent unified directory structure
- Files:
  - `~/.config/slack-rs/profiles.json` - Profile metadata (non-secret)
- Secrets (tokens, OAuth client secrets) live in the OS keyring, not files
- Legacy paths (automatically migrated):
  - macOS: `~/Library/Application Support/slack-rs/profiles.json`
  - Linux: `~/.config/slack-rs/profiles.json`
  - Windows: `%APPDATA%\slack-rs\profiles.json`

## profiles.json Schema

```json
{
  "version": 1,
  "profiles": [
    {
      "profile_name": "acme-work",
      "team_id": "T123ABC",
      "team_name": "Acme Corp",
      "user_id": "U456DEF",
      "user_name": "john.doe",
      "scopes": [
        "search:read",
        "channels:read",
        "channels:history",
        "users:read",
        "chat:write"
      ],
      "created_at": "2026-02-03T10:30:00Z",
      "last_used_at": "2026-02-03T15:45:00Z"
    }
  ]
}
```

### Field Definitions
- `version`: Schema version (currently `1`)
- `profiles`: Array of profile objects
  - `profile_name`: User-chosen alias (must be unique)
  - `team_id`: Slack workspace ID (from `oauth.v2.access`)
  - `team_name`: Slack workspace name (from `oauth.v2.access`)
  - `user_id`: Authenticated user ID (from `oauth.v2.access`)
  - `user_name`: Optional user name (can be fetched via `users.info` later)
  - `scopes`: Array of granted OAuth scopes
  - `created_at`: ISO 8601 timestamp of profile creation
  - `last_used_at`: ISO 8601 timestamp of last command execution

### Constraints
- `profile_name` must be unique across all profiles
- `(team_id, user_id)` combination should be unique (enforced on login)
- If a duplicate `(team_id, user_id)` is detected during login:
  - Update existing profile's token and metadata
  - Do not create a new profile entry

## Keyring Token Storage

### Storage Location
- OS keyring, service name `slack` (kept in sync with `config/cli.toml`)
- Backends: macOS Keychain, Windows Credential Manager, Linux Secret
  Service (gnome-keyring / KWallet via D-Bus)
- No plaintext token files exist; `profiles.json` holds only non-secret
  metadata and doubles as the profile index (the keyring cannot enumerate
  entries)

### Entry Layout
Each keyring entry holds a JSON map of token-store keys to secret values:

- Account `{team_id}:{user_id}` — bot and user tokens for one profile
  identity share one entry:

```json
{
  "T123ABC:U456DEF": "xoxb-...",
  "T123ABC:U456DEF:user": "xoxp-..."
}
```

- Account `oauth-client-secret:{profile_name}` — the profile's OAuth
  client secret

### Key Structure (unchanged from the legacy file format)
- **Profile tokens**: `{team_id}:{user_id}` (example: `T123ABC:U456DEF`)
- **Scoped tokens**: `{team_id}:{user_id}:{scope}` (example: `T123ABC:U456DEF:user`)
- **OAuth secrets**: `oauth-client-secret:{profile_name}` (example: `oauth-client-secret:default`)

### Migration from Legacy tokens.json
- `slack auth migrate [--path <file>]` imports every key from a legacy
  plaintext `tokens.json` (default: `~/.local/share/slack-rs/tokens.json`)
  into the keyring, then overwrites the file with zeros and deletes it
- Commands that fail to find tokens print a migration hint when a legacy
  file is detected

### Security Considerations
- Secrets never touch disk unencrypted; storage is delegated to the OS
  credential store
- Headless Linux requires a running Secret Service provider
  (e.g. `gnome-keyring-daemon`) on the session D-Bus
- Encrypted export/import (`auth export` / `auth import`) is the supported
  way to move credentials between machines

## Profile Resolution Flow

1. User runs: `slack --profile acme-work search "query"`
2. CLI reads `~/.config/slack-rs/profiles.json`
3. Find profile with `profile_name == "acme-work"`
4. Extract `team_id` and `user_id`
5. Construct token key: `{team_id}:{user_id}`
6. Retrieve token from the OS keyring (entry `{team_id}:{user_id}`)
7. Execute API call with token
8. Update `last_used_at` in `profiles.json`

## Error Handling

### Profile Not Found
- Error: `Profile 'xyz' not found. Run 'slack auth list' to see available profiles.`
- Exit code: 1

### Token Not Found in Storage
- Error: `Token not found for profile 'xyz'. Run 'slack auth login --profile xyz' to re-authenticate.`
- Exit code: 1

### Config File Corruption
- Error: `Failed to parse profiles.json: {error}. Consider backing up and deleting the file.`
- Exit code: 1

### Keyring Unavailable
- Error: `OS keyring unavailable: {error}` plus platform-specific guidance
  (on Linux: install/start a Secret Service provider such as gnome-keyring)
- Exit code: 1

## Migration Strategy
- If `profiles.json` does not exist at the new path: check for legacy config and migrate
  - Legacy config path: `ProjectDirs::from("", "", "slack-rs")` + `profiles.json`
  - Migration: Try `fs::rename` first; if it fails, copy content and keep old file
  - Migration is automatic and transparent on first access
- If `profiles.json` does not exist: create with `version: 1` and empty `profiles` array
- If `version` field is missing or < 1: attempt to migrate (future-proofing)
- Always validate schema on load; fail fast on corruption
