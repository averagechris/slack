# slack-rs (skill)

Agent skill for Slack Web API automation using the `slack` CLI.

This directory contains the skill documentation only (no bundled scripts/binaries). The `slack` CLI is a separate project:

- https://git.sr.ht/~averagechris/slack (fork of https://github.com/tumf/slack-rs)

## What You Get

- `slack-rs/SKILL.md`: day-to-day agent instructions for using `slack`
- `slack-rs/references/setup.md`: one-time setup and onboarding steps
- `slack-rs/references/recipes.md`: copy/paste operational recipes

## Install The Skill

Recommended:

```bash
npx skills add tumf/skills --skill slack-rs
```

Alternative: load the skill file directly in your agent config:

```jsonc
{
  "instructions": ["path/to/slack-rs/SKILL.md"]
}
```

## Prerequisites

Install the `slack` CLI on the machine where the agent runs.

This skill assumes recent versions of `slack` (v0.1.40+):

```bash
slack --version
slack --help
```

Tip: `slack` supports machine-readable introspection:

```bash
slack commands --json
slack conv list --help --json
slack schema --command msg.post --output json-schema
```

## Using The Skill

Once the skill is loaded, the agent will run `slack` commands directly.

For first-time setup, read `slack-rs/references/setup.md`.

Prefer convenience commands when possible:

```bash
slack conv list
slack conv search <pattern>
slack conv history <channel_id>
slack thread get <channel_id> <thread_ts>

slack msg post <channel_id> "Hello"
```

For anything else, use the generic method runner:

```bash
slack api call <method> [params...]
```

## Credentials And Storage

`slack` stores profiles, OAuth config, and tokens under `~/.config/slack-rs/`. Treat this directory as a secret.

For backup/migration, refer to:

```bash
slack auth export --help
slack auth import --help
```

## Write Safety Guard

Many Slack methods write data (posting, updating, deleting, reactions). In automation shells, set:

```bash
export SLACKCLI_ALLOW_WRITE=false
```

Enable writes only when you intend to change Slack:

```bash
export SLACKCLI_ALLOW_WRITE=true
```
