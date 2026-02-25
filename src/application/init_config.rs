use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

use crate::application::config::{
    InitConfigArgs, InitScope, system_config_toml_path, user_config_toml_path,
};

#[derive(Debug, Clone)]
struct InitPaths {
    user_config_toml: Option<PathBuf>,
    system_config_toml: PathBuf,
}

impl InitPaths {
    fn from_environment() -> Self {
        Self {
            user_config_toml: user_config_toml_path(),
            system_config_toml: system_config_toml_path(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetScope {
    User,
    System,
}

#[derive(Debug, Clone)]
struct ConfigTarget {
    scope: TargetScope,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteOutcome {
    Created,
    Overwritten,
    Skipped,
}

pub fn run(args: &InitConfigArgs) -> Result<(), String> {
    let interactive = should_interact(args.non_interactive);
    let paths = InitPaths::from_environment();
    let targets = resolve_targets(args.scope, &paths)?;

    let mut created = 0usize;
    let mut overwritten = 0usize;
    let mut skipped = 0usize;

    for target in targets {
        let outcome = ensure_config_file(target, args.force, interactive)?;
        match outcome {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Overwritten => overwritten += 1,
            WriteOutcome::Skipped => skipped += 1,
        }
    }

    println!(
        "init-config complete (created={created}, overwritten={overwritten}, skipped={skipped})"
    );

    Ok(())
}

fn resolve_targets(scope: InitScope, paths: &InitPaths) -> Result<Vec<ConfigTarget>, String> {
    match scope {
        InitScope::User => {
            let user = paths.user_config_toml.clone().ok_or_else(|| {
                "unable to resolve user home directory for config path".to_owned()
            })?;
            Ok(vec![ConfigTarget {
                scope: TargetScope::User,
                path: user,
            }])
        }
        InitScope::System => Ok(vec![ConfigTarget {
            scope: TargetScope::System,
            path: paths.system_config_toml.clone(),
        }]),
        InitScope::Both => {
            let user = paths.user_config_toml.clone().ok_or_else(|| {
                "unable to resolve user home directory for config path".to_owned()
            })?;
            Ok(vec![
                ConfigTarget {
                    scope: TargetScope::System,
                    path: paths.system_config_toml.clone(),
                },
                ConfigTarget {
                    scope: TargetScope::User,
                    path: user,
                },
            ])
        }
    }
}

fn ensure_config_file(
    target: ConfigTarget,
    force: bool,
    interactive: bool,
) -> Result<WriteOutcome, String> {
    if target.path.exists() {
        if !force {
            println!(
                "{} config already exists at {}; skipping",
                target.scope.label(),
                target.path.display()
            );
            return Ok(WriteOutcome::Skipped);
        }

        write_template_file(&target)?;
        println!(
            "{} config overwritten at {}",
            target.scope.label(),
            target.path.display()
        );
        return Ok(WriteOutcome::Overwritten);
    }

    if interactive {
        let question = format!(
            "Create {} config at {}? [Y/n]: ",
            target.scope.label(),
            target.path.display()
        );
        let approved = prompt_yes_no(&question, true)?;
        if !approved {
            println!(
                "{} config creation declined for {}",
                target.scope.label(),
                target.path.display()
            );
            return Ok(WriteOutcome::Skipped);
        }
    }

    write_template_file(&target)?;
    println!(
        "{} config created at {}",
        target.scope.label(),
        target.path.display()
    );
    Ok(WriteOutcome::Created)
}

fn write_template_file(target: &ConfigTarget) -> Result<(), String> {
    if let Some(parent) = target.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    let content = base_config_template(target);
    fs::write(&target.path, content)
        .map_err(|error| format!("failed to write {}: {error}", target.path.display()))
}

fn base_config_template(target: &ConfigTarget) -> String {
    let db_path = suggested_db_path(target);

    format!(
        "# Reclaw Core static runtime configuration\n\
# Compatible with OpenClaw protocol semantics.\n\
# Precedence: defaults < static files < env/CLI.\n\
#\n\
# Supported static file search order:\n\
#   1. /etc/reclaw/config.toml\n\
#   2. /etc/reclaw/config.json\n\
#   3. ~/.reclaw/config.toml\n\
#   4. ~/.reclaw/config.json\n\
\n\
host = \"127.0.0.1\"\n\
port = 18789\n\
cronEnabled = true\n\
logFilter = \"info\"\n\
jsonLogs = false\n\
dbPath = \"{}\"\n\
\n\
# Set only one of gatewayToken or gatewayPassword.\n\
# gatewayToken = \"replace-me\"\n\
# gatewayPassword = \"replace-me\"\n\
\n\
# Optional bearer token for /channels/inbound (recommended when exposed).\n\
# channelsInboundToken = \"replace-me\"\n\
\n\
# Telegram webhook integration (optional).\n\
# telegramWebhookSecret = \"replace-me\"\n\
# telegramBotToken = \"replace-me\"\n\
# telegramApiBaseUrl = \"https://api.telegram.org\"\n\
\n\
# Additional channel webhook tokens (Authorization: Bearer <token>).\n\
# discordWebhookToken = \"replace-me\"\n\
# slackWebhookToken = \"replace-me\"\n\
# signalWebhookToken = \"replace-me\"\n\
# whatsappWebhookToken = \"replace-me\"\n\
\n\
# HTTP compatibility endpoints (disabled by default).\n\
# openaiChatCompletionsEnabled = true\n\
# openresponsesEnabled = true\n",
        db_path.display()
    )
}

fn suggested_db_path(target: &ConfigTarget) -> PathBuf {
    match target.scope {
        TargetScope::System => PathBuf::from("/var/lib/reclaw/reclaw.db"),
        TargetScope::User => target
            .path
            .parent()
            .map(|parent| parent.join("reclaw.db"))
            .unwrap_or_else(|| PathBuf::from("./.reclaw-core/reclaw.db")),
    }
}

fn should_interact(non_interactive: bool) -> bool {
    !non_interactive && io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_yes_no(question: &str, default_yes: bool) -> Result<bool, String> {
    loop {
        print!("{question}");
        io::stdout()
            .flush()
            .map_err(|error| format!("failed to flush prompt: {error}"))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|error| format!("failed to read prompt input: {error}"))?;

        let normalized = input.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Ok(default_yes);
        }
        if normalized == "y" || normalized == "yes" {
            return Ok(true);
        }
        if normalized == "n" || normalized == "no" {
            return Ok(false);
        }

        println!("please answer yes or no");
    }
}

impl TargetScope {
    fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::application::{
        config::{InitConfigArgs, InitScope},
        init_config::{
            ConfigTarget, InitPaths, TargetScope, WriteOutcome, ensure_config_file, resolve_targets,
        },
    };

    #[test]
    fn resolve_targets_requires_home_for_user_scope() {
        let paths = InitPaths {
            user_config_toml: None,
            system_config_toml: std::path::PathBuf::from("/etc/reclaw/config.toml"),
        };

        let result = resolve_targets(InitScope::User, &paths);
        assert!(result.is_err());
    }

    #[test]
    fn ensure_config_file_creates_missing_user_template() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let target = ConfigTarget {
            scope: TargetScope::User,
            path: temp.path().join(".reclaw/config.toml"),
        };

        let result = ensure_config_file(target.clone(), false, false).expect("init should succeed");
        assert_eq!(result, WriteOutcome::Created);
        assert!(target.path.exists());

        let contents = fs::read_to_string(&target.path).expect("config should be readable");
        assert!(contents.contains("dbPath"));
        assert!(contents.contains(".reclaw/reclaw.db"));
    }

    #[test]
    fn ensure_config_file_skips_existing_without_force() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let path = temp.path().join("config.toml");
        fs::write(&path, "port = 20000\n").expect("seed config should be written");

        let target = ConfigTarget {
            scope: TargetScope::User,
            path: path.clone(),
        };

        let result = ensure_config_file(target, false, false).expect("init should succeed");
        assert_eq!(result, WriteOutcome::Skipped);

        let contents = fs::read_to_string(path).expect("config should be readable");
        assert_eq!(contents, "port = 20000\n");
    }

    #[test]
    fn ensure_config_file_overwrites_when_force_is_enabled() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let path = temp.path().join("config.toml");
        fs::write(&path, "port = 20000\n").expect("seed config should be written");

        let target = ConfigTarget {
            scope: TargetScope::System,
            path: path.clone(),
        };

        let result = ensure_config_file(target, true, false).expect("init should succeed");
        assert_eq!(result, WriteOutcome::Overwritten);

        let contents = fs::read_to_string(path).expect("config should be readable");
        assert!(contents.contains("/var/lib/reclaw/reclaw.db"));
    }

    #[test]
    fn resolve_targets_both_scope_orders_system_then_user() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let args = InitConfigArgs {
            scope: InitScope::Both,
            non_interactive: true,
            force: false,
        };

        let paths = InitPaths {
            user_config_toml: Some(temp.path().join("user/config.toml")),
            system_config_toml: temp.path().join("etc/config.toml"),
        };

        let targets = resolve_targets(args.scope, &paths).expect("targets should resolve");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].scope, TargetScope::System);
        assert_eq!(targets[1].scope, TargetScope::User);
    }
}
