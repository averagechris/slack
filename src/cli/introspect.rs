//! CLI introspection - self-describing CLI capabilities.
//!
//! Provides machine-readable information about commands, flags, and output
//! schemas. Unlike the pre-clap hand-mirrored registry, everything here is
//! generated at runtime from the clap command model in [`crate::cli::args`],
//! so it cannot drift from the actual parser.

use clap::CommandFactory;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::args::Cli;

/// CLI command definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDef {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub flags: Vec<FlagDef>,
    pub exit_codes: Vec<ExitCodeDef>,
}

/// Flag/option/positional definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagDef {
    pub name: String,
    #[serde(rename = "type")]
    pub flag_type: String,
    pub required: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub global: bool,
}

/// Exit code definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitCodeDef {
    pub code: i32,
    pub description: String,
}

/// Commands list response
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandsListResponse {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub response_type: String,
    pub ok: bool,
    pub commands: Vec<CommandDef>,
}

/// Structured help response
#[derive(Debug, Serialize, Deserialize)]
pub struct HelpResponse {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub response_type: String,
    pub ok: bool,
    pub command: String,
    pub usage: String,
    pub flags: Vec<FlagDef>,
    #[serde(rename = "exitCodes")]
    pub exit_codes: Vec<ExitCodeDef>,
}

/// Schema response
#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaResponse {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub response_type: String,
    pub ok: bool,
    pub command: String,
    pub schema: Value,
}

/// Standard exit code contract shared by all commands.
fn standard_exit_codes() -> Vec<ExitCodeDef> {
    vec![
        ExitCodeDef {
            code: 0,
            description: "Success".to_string(),
        },
        ExitCodeDef {
            code: 1,
            description: "Error (including usage errors)".to_string(),
        },
        ExitCodeDef {
            code: 2,
            description: "Interactive input required but running non-interactively".to_string(),
        },
    ]
}

/// Map a clap argument to a FlagDef.
fn arg_to_flag_def(arg: &clap::Arg) -> FlagDef {
    let name = if let Some(long) = arg.get_long() {
        format!("--{}", long)
    } else if let Some(short) = arg.get_short() {
        format!("-{}", short)
    } else {
        // Positional argument: report its value name / id.
        arg.get_id().to_string()
    };

    let flag_type = if !arg.get_action().takes_values() {
        "boolean".to_string()
    } else {
        let type_id = arg.get_value_parser().type_id();
        if type_id == clap::builder::ValueParser::new(clap::value_parser!(u32)).type_id() {
            "integer".to_string()
        } else {
            "string".to_string()
        }
    };

    let default = arg
        .get_default_values()
        .first()
        .map(|v| v.to_string_lossy().to_string());

    FlagDef {
        name,
        flag_type,
        required: arg.is_required_set(),
        description: arg
            .get_help()
            .map(|h| h.to_string())
            .unwrap_or_default()
            .lines()
            .collect::<Vec<_>>()
            .join(" "),
        default,
        global: arg.is_global_set(),
    }
}

/// Build a CommandDef for a leaf command at the given path.
fn command_def(path: &[&str], cmd: &clap::Command) -> CommandDef {
    let mut usage_cmd = cmd.clone();
    let usage = usage_cmd
        .render_usage()
        .to_string()
        .trim_start_matches("Usage:")
        .trim()
        .to_string();

    let flags = cmd
        .get_arguments()
        .filter(|a| {
            let id = a.get_id().as_str();
            id != "help" && id != "version"
        })
        .map(arg_to_flag_def)
        .collect();

    CommandDef {
        name: path.join(" "),
        description: cmd
            .get_about()
            .map(|a| a.to_string())
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        usage,
        flags,
        exit_codes: standard_exit_codes(),
    }
}

/// Recursively collect leaf command definitions.
fn collect_commands(path: Vec<&str>, cmd: &clap::Command, out: &mut Vec<CommandDef>) {
    let subcommands: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .collect();

    if subcommands.is_empty() && !path.is_empty() {
        out.push(command_def(&path, cmd));
        return;
    }

    for sub in subcommands {
        let mut sub_path = path.clone();
        sub_path.push(sub.get_name());
        collect_commands(sub_path, sub, out);
    }
}

/// Build the full (propagated) clap command model.
fn full_command() -> clap::Command {
    let mut cmd = Cli::command();
    // Ensure global args and settings are propagated into subcommands
    cmd.build();
    cmd
}

/// Get all leaf command definitions from the clap model.
pub fn get_command_definitions() -> Vec<CommandDef> {
    let cmd = full_command();
    let mut out = Vec::new();
    collect_commands(Vec::new(), &cmd, &mut out);
    out
}

/// Find a subcommand by space- or dot-separated path (e.g. "conv list" / "conv.list").
fn find_subcommand(command_name: &str) -> Option<clap::Command> {
    let normalized = command_name.replace('.', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let mut current = full_command();
    for part in &parts {
        let next = current.get_subcommands().find(|c| c.get_name() == *part)?;
        current = next.clone();
    }
    Some(current)
}

/// Get command definition by name.
/// Supports both space-separated ("conv list") and dot-separated ("conv.list") formats.
pub fn get_command_definition(command_name: &str) -> Option<CommandDef> {
    let normalized = command_name.replace('.', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    let cmd = find_subcommand(command_name)?;
    Some(command_def(&parts, &cmd))
}

/// Generate commands list response.
pub fn generate_commands_list() -> CommandsListResponse {
    CommandsListResponse {
        schema_version: 1,
        response_type: "commands.list".to_string(),
        ok: true,
        commands: get_command_definitions(),
    }
}

/// Generate structured help for a command.
pub fn generate_help(command_name: &str) -> Result<HelpResponse, String> {
    let cmd = get_command_definition(command_name)
        .ok_or_else(|| format!("Command '{}' not found", command_name))?;

    Ok(HelpResponse {
        schema_version: 1,
        response_type: "help".to_string(),
        ok: true,
        command: cmd.name.clone(),
        usage: cmd.usage.clone(),
        flags: cmd.flags.clone(),
        exit_codes: cmd.exit_codes.clone(),
    })
}

/// Generate JSON schema for a command's output.
pub fn generate_schema(command_name: &str) -> Result<SchemaResponse, String> {
    // Verify the command exists in the clap model
    let _cmd = get_command_definition(command_name)
        .ok_or_else(|| format!("Command '{}' not found", command_name))?;

    // Special case for install-skills
    let schema = if command_name == "install-skills" {
        serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "schemaVersion": {
                    "type": "string",
                    "description": "Schema version number"
                },
                "type": {
                    "type": "string",
                    "description": "Response type identifier",
                    "const": "skill-installation"
                },
                "ok": {
                    "type": "boolean",
                    "description": "Indicates if the operation was successful"
                },
                "skills": {
                    "type": "array",
                    "description": "List of installed skills",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Skill name"
                            },
                            "path": {
                                "type": "string",
                                "description": "Installation path"
                            },
                            "source_type": {
                                "type": "string",
                                "description": "Source type (self or local)"
                            }
                        },
                        "required": ["name", "path", "source_type"]
                    }
                }
            },
            "required": ["schemaVersion", "type", "ok", "skills"]
        })
    } else {
        // Generic envelope schema for all other commands
        serde_json::json!({
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
        })
    };

    Ok(SchemaResponse {
        schema_version: 1,
        response_type: "schema".to_string(),
        ok: true,
        command: command_name.to_string(),
        schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_command_definitions() {
        let commands = get_command_definitions();
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| c.name == "api call"));
        assert!(commands.iter().any(|c| c.name == "conv list"));
        assert!(commands.iter().any(|c| c.name == "auth migrate"));
        assert!(commands.iter().any(|c| c.name == "completions"));
        assert!(commands.iter().any(|c| c.name == "install-skills"));
    }

    #[test]
    fn test_no_intermediate_group_commands() {
        let commands = get_command_definitions();
        // Group commands like "conv" / "auth" must not appear as leaves
        assert!(!commands.iter().any(|c| c.name == "conv"));
        assert!(!commands.iter().any(|c| c.name == "auth"));
    }

    #[test]
    fn test_get_command_definition() {
        let cmd = get_command_definition("conv list").unwrap();
        assert_eq!(cmd.name, "conv list");
        assert!(!cmd.flags.is_empty());
        assert!(cmd.flags.iter().any(|f| f.name == "--filter"));
        assert!(cmd.flags.iter().any(|f| f.name == "--profile" && f.global));
        // dot-separated lookup
        assert!(get_command_definition("conv.list").is_some());
        assert!(get_command_definition("nope nope").is_none());
    }

    #[test]
    fn test_flag_types_and_defaults() {
        let cmd = get_command_definition("conv list").unwrap();
        let limit = cmd.flags.iter().find(|f| f.name == "--limit").unwrap();
        assert_eq!(limit.flag_type, "integer");
        let raw = cmd.flags.iter().find(|f| f.name == "--raw").unwrap();
        assert_eq!(raw.flag_type, "boolean");
        let types = cmd.flags.iter().find(|f| f.name == "--types").unwrap();
        assert_eq!(types.flag_type, "string");
    }

    #[test]
    fn test_positionals_included() {
        let cmd = get_command_definition("thread get").unwrap();
        assert!(cmd.flags.iter().any(|f| f.name == "channel" && f.required));
        assert!(cmd
            .flags
            .iter()
            .any(|f| f.name == "thread_ts" && f.required));
    }

    #[test]
    fn test_generate_commands_list() {
        let response = generate_commands_list();
        assert_eq!(response.schema_version, 1);
        assert_eq!(response.response_type, "commands.list");
        assert!(response.ok);
        assert!(!response.commands.is_empty());
    }

    #[test]
    fn test_generate_help() {
        let help = generate_help("conv list").unwrap();
        assert_eq!(help.schema_version, 1);
        assert_eq!(help.response_type, "help");
        assert!(help.ok);
        assert_eq!(help.command, "conv list");
        assert!(help.usage.contains("conv list"));
    }

    #[test]
    fn test_generate_help_unknown_command() {
        assert!(generate_help("unknown command").is_err());
    }

    #[test]
    fn test_generate_schema() {
        let schema = generate_schema("conv list").unwrap();
        assert_eq!(schema.schema_version, 1);
        assert_eq!(schema.response_type, "schema");
        assert!(schema.ok);
        assert_eq!(schema.command, "conv list");
    }

    #[test]
    fn test_json_serialization_roundtrip() {
        let response = generate_commands_list();
        let json = serde_json::to_string(&response).unwrap();
        let parsed: CommandsListResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);

        let help = generate_help("conv list").unwrap();
        let json = serde_json::to_string(&help).unwrap();
        let parsed: HelpResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);

        let schema = generate_schema("conv list").unwrap();
        let json = serde_json::to_string(&schema).unwrap();
        let parsed: SchemaResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);
    }
}
