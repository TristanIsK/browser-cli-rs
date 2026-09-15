use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::{Args, Parser, Subcommand};
use lexmount_browser::{
    Client, Error, Result, auth,
    cdp::{Cdp, WaitTextOptions},
    models::CreateSession,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "browser-cli",
    version,
    about = "Lexmount cloud browser CLI (native Rust)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Version,
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Doctor,
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    Action {
        /// Operate on this page target from `session targets`, not the default page.
        #[arg(long, global = true)]
        target_id: Option<String>,
        #[command(subcommand)]
        command: ActionCommand,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    Status,
    Login {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long, default_value = auth::DEFAULT_CLIENT_NAME)]
        client_name: String,
        #[arg(long, default_value = auth::DEFAULT_CONNECT_BASE_URL)]
        connect_base_url: String,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        credentials_file: Option<PathBuf>,
    },
    Logout {
        #[arg(long)]
        credentials_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionCommand {
    Create(CreateSessionArgs),
    Get {
        #[arg(long)]
        session_id: String,
    },
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Close {
        #[arg(long)]
        session_id: String,
    },
    Keepalive {
        #[arg(long)]
        session_id: String,
        #[arg(long, default_value_t = 5)]
        interval: u64,
        #[arg(long, default_value_t = 60)]
        duration: u64,
        #[arg(long)]
        stop_on_inactive: bool,
    },
    Targets {
        #[arg(long)]
        session_id: String,
    },
    Downloads {
        #[command(subcommand)]
        command: DownloadsCommand,
    },
}

#[derive(Subcommand)]
enum DownloadsCommand {
    List {
        #[arg(long)]
        session_id: String,
    },
    Get {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        download_id: String,
        #[arg(long)]
        output: PathBuf,
    },
    Archive {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        output: PathBuf,
    },
    Delete {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
struct CreateSessionArgs {
    #[arg(long, default_value = "normal")]
    browser_mode: String,
    #[arg(long)]
    context_id: Option<String>,
    #[arg(long, default_value = "read_write")]
    context_mode: String,
    #[arg(long)]
    context_description: Option<String>,
    #[arg(long)]
    weak_lock: bool,
    #[arg(long)]
    official_proxy: bool,
    #[arg(long)]
    custom_image_id: Option<String>,
    #[arg(long)]
    window_size: Option<String>,
    #[arg(long)]
    downloads: bool,
    #[arg(long)]
    recording: bool,
    #[arg(long, default_value_t = 600)]
    timeout_seconds: u64,
}

#[derive(Subcommand)]
enum ContextCommand {
    Create {
        #[arg(long)]
        metadata_json: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    Get {
        #[arg(long)]
        context_id: String,
    },
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    Fork {
        #[arg(long)]
        context_id: String,
    },
    Delete {
        #[arg(long)]
        context_id: String,
        #[arg(long)]
        yes: bool,
    },
    ForceRelease {
        #[arg(long)]
        context_id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ActionCommand {
    OpenUrl {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        url: String,
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
    Eval {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        expression: String,
    },
    WaitSelector {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        selector: String,
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
    WaitText {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long, value_parser = ["present", "absent"], default_value = "present")]
        state: String,
        #[arg(long)]
        exact: bool,
        #[arg(long)]
        case_sensitive: bool,
        #[arg(long)]
        include_hidden: bool,
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 250)]
        poll_ms: u64,
    },
    Click {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        selector: String,
    },
    Fill {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        selector: String,
        #[arg(long)]
        value: String,
    },
    Screenshot {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        full_page: bool,
    },
    Pdf {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        print_background: bool,
    },
    Snapshot {
        #[arg(long)]
        session_id: String,
    },
    Raw {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params_json: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(value) => {
            print_json(&json!({"ok": true, "data": value}));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string(
                    &json!({"ok": false, "error": error_kind(&error), "message": error.to_string()})
                )
                .unwrap()
            );
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<Value> {
    match cli.command {
        Command::Version => {
            Ok(json!({"name":"browser-cli","version":env!("CARGO_PKG_VERSION"),"runtime":"rust"}))
        }
        Command::Auth { command } => run_auth(command),
        Command::Doctor => doctor(),
        Command::Session { command } => run_session(Client::from_env()?, command),
        Command::Context { command } => run_context(Client::from_env()?, command),
        Command::Action { command, target_id } => {
            run_action(Client::from_env()?, command, target_id.as_deref())
        }
    }
}

fn run_auth(command: AuthCommand) -> Result<Value> {
    match command {
        AuthCommand::Status => Ok(serde_json::to_value(auth::status(None)?)?),
        AuthCommand::Login {
            project_id,
            client_name,
            connect_base_url,
            timeout_seconds,
            no_open,
            credentials_file,
        } => auth::login_with_client_name(
            project_id.as_deref(),
            &client_name,
            &connect_base_url,
            Duration::from_secs(timeout_seconds),
            !no_open,
            credentials_file.as_deref(),
        ),
        AuthCommand::Logout { credentials_file } => {
            Ok(json!({"removed":auth::logout(credentials_file.as_deref())?}))
        }
    }
}

fn doctor() -> Result<Value> {
    let status = auth::status(None)?;
    let mut checks = vec![
        json!({"id":"credentials","ok":status.valid,"source":status.source,"path":status.path}),
    ];
    let api = if status.valid {
        match Client::from_env().and_then(|client| client.list_sessions(Some("active"))) {
            Ok(_) => {
                checks.push(json!({"id":"api","ok":true}));
                true
            }
            Err(error) => {
                checks.push(json!({"id":"api","ok":false,"message":error.to_string()}));
                false
            }
        }
    } else {
        checks.push(json!({"id":"api","ok":false,"skipped":true,"message":"credentials are not configured"}));
        false
    };
    Ok(
        json!({"ready_for_browser_actions":status.valid && api,"checks":checks,"fix":if status.valid {""} else {"Run `browser-cli auth login`."}}),
    )
}

fn run_session(client: Client, command: SessionCommand) -> Result<Value> {
    match command {
        SessionCommand::Create(args) => {
            let context = args
                .context_id
                .map(|id| json!({"id":id,"mode":args.context_mode}));
            let request = CreateSession {
                browser_mode: args.browser_mode,
                context,
                context_description: args.context_description,
                weak_lock: args.weak_lock.then_some(true),
                official_proxy: args.official_proxy.then_some(true),
                custom_image_id: args.custom_image_id,
                window_size: args.window_size,
                downloads: args.downloads.then_some(json!({"enabled":true})),
                recording: args.recording.then_some(json!({"persistent":true})),
                ..Default::default()
            };
            Ok(serde_json::to_value(client.create_session(
                request,
                Duration::from_secs(args.timeout_seconds),
            )?)?)
        }
        SessionCommand::Get { session_id } => {
            Ok(serde_json::to_value(client.get_session(&session_id)?)?)
        }
        SessionCommand::List { status } => Ok(serde_json::to_value(
            client.list_sessions(status.as_deref())?,
        )?),
        SessionCommand::Close { session_id } => {
            client.close_session(&session_id)?;
            Ok(json!({"session_id":session_id,"closed":true}))
        }
        SessionCommand::Targets { session_id } => client.session_targets(&session_id),
        SessionCommand::Downloads { command } => match command {
            DownloadsCommand::List { session_id } => client.list_downloads(&session_id),
            DownloadsCommand::Get {
                session_id,
                download_id,
                output,
            } => {
                let bytes = client.get_download(&session_id, &download_id)?;
                std::fs::write(&output, &bytes)?;
                Ok(
                    json!({"session_id":session_id,"download_id":download_id,"output":output,"bytes":bytes.len()}),
                )
            }
            DownloadsCommand::Archive { session_id, output } => {
                let bytes = client.archive_downloads(&session_id)?;
                std::fs::write(&output, &bytes)?;
                Ok(json!({"session_id":session_id,"output":output,"bytes":bytes.len()}))
            }
            DownloadsCommand::Delete { session_id, yes } => {
                require_yes(yes, "session downloads delete")?;
                client.delete_downloads(&session_id)
            }
        },
        SessionCommand::Keepalive {
            session_id,
            interval,
            duration,
            stop_on_inactive,
        } => {
            let started = std::time::Instant::now();
            let mut snapshots = vec![];
            loop {
                let session = client.get_session(&session_id)?;
                let inactive = session.status != "active";
                snapshots.push(serde_json::to_value(&session)?);
                if duration == 0
                    || started.elapsed() >= Duration::from_secs(duration)
                    || (inactive && stop_on_inactive)
                {
                    break;
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
            Ok(json!({"session_id":session_id,"checks":snapshots.len(),"snapshots":snapshots}))
        }
    }
}

fn run_context(client: Client, command: ContextCommand) -> Result<Value> {
    match command {
        ContextCommand::Create {
            metadata_json,
            description,
        } => {
            let metadata = metadata_json
                .map(|v| serde_json::from_str(&v))
                .transpose()?;
            Ok(serde_json::to_value(
                client.create_context(metadata, description.as_deref())?,
            )?)
        }
        ContextCommand::Get { context_id } => {
            Ok(serde_json::to_value(client.get_context(&context_id)?)?)
        }
        ContextCommand::List { status, limit } => Ok(serde_json::to_value(
            client.list_contexts(status.as_deref(), limit)?,
        )?),
        ContextCommand::Fork { context_id } => {
            Ok(serde_json::to_value(client.fork_context(&context_id)?)?)
        }
        ContextCommand::Delete { context_id, yes } => {
            require_yes(yes, "context delete")?;
            client.delete_context(&context_id)?;
            Ok(json!({"context_id":context_id,"deleted":true}))
        }
        ContextCommand::ForceRelease { context_id, yes } => {
            require_yes(yes, "context force-release")?;
            client.force_release_context(&context_id)
        }
    }
}

fn run_action(client: Client, command: ActionCommand, target_id: Option<&str>) -> Result<Value> {
    let session_id = match &command {
        ActionCommand::OpenUrl { session_id, .. }
        | ActionCommand::Eval { session_id, .. }
        | ActionCommand::WaitSelector { session_id, .. }
        | ActionCommand::WaitText { session_id, .. }
        | ActionCommand::Click { session_id, .. }
        | ActionCommand::Fill { session_id, .. }
        | ActionCommand::Screenshot { session_id, .. }
        | ActionCommand::Pdf { session_id, .. }
        | ActionCommand::Snapshot { session_id }
        | ActionCommand::Raw { session_id, .. } => session_id,
    };
    let session = client.get_session_with_ws(session_id)?;
    let ws = session
        .ws
        .ok_or_else(|| Error::Cdp(format!("session {session_id} has no CDP WebSocket URL")))?;
    let mut cdp = match target_id {
        Some(id) => Cdp::connect_to_target(&ws, id)?,
        None => Cdp::connect(&ws)?,
    };
    match command {
        ActionCommand::OpenUrl {
            url, timeout_ms, ..
        } => cdp.navigate(&url, Duration::from_millis(timeout_ms)),
        ActionCommand::Eval { expression, .. } => cdp.evaluate(&expression),
        ActionCommand::WaitSelector {
            selector,
            timeout_ms,
            ..
        } => cdp.wait_selector(&selector, Duration::from_millis(timeout_ms)),
        ActionCommand::WaitText {
            text,
            selector,
            state,
            exact,
            case_sensitive,
            include_hidden,
            timeout_ms,
            poll_ms,
            ..
        } => cdp.wait_text(WaitTextOptions {
            text: &text,
            selector: selector.as_deref(),
            state: &state,
            exact,
            case_sensitive,
            include_hidden,
            timeout: Duration::from_millis(timeout_ms),
            poll: Duration::from_millis(poll_ms),
        }),
        ActionCommand::Click { selector, .. } => cdp.click(&selector),
        ActionCommand::Fill {
            selector, value, ..
        } => cdp.fill(&selector, &value),
        ActionCommand::Screenshot {
            path, full_page, ..
        } => cdp.screenshot(&path, full_page),
        ActionCommand::Pdf {
            path,
            print_background,
            ..
        } => cdp.pdf(&path, print_background),
        ActionCommand::Snapshot { .. } => cdp.snapshot(),
        ActionCommand::Raw {
            method,
            params_json,
            ..
        } => cdp.command(&method, serde_json::from_str(&params_json)?),
    }
}

fn require_yes(yes: bool, action: &str) -> Result<()> {
    if yes {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "{action} is destructive; rerun with --yes"
        )))
    }
}
fn error_kind(error: &Error) -> &'static str {
    match error {
        Error::Config(_) => "configuration_error",
        Error::Authentication(_) => "authentication_error",
        Error::NotFound(_) => "not_found",
        Error::Conflict(_) => "conflict",
        Error::Timeout(_) => "timeout",
        Error::Api { .. } => "api_error",
        Error::Http(_) => "http_error",
        Error::WebSocket(_) => "websocket_error",
        Error::Json(_) => "json_error",
        Error::Io(_) => "io_error",
        Error::Cdp(_) => "cdp_error",
    }
}
fn print_json<T: Serialize>(value: &T) {
    println!("{}", serde_json::to_string(value).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_target_flag_is_accepted_before_or_after_action_subcommand() {
        for args in [
            vec![
                "browser-cli",
                "action",
                "--target-id",
                "result",
                "eval",
                "--session-id",
                "browser",
                "--expression",
                "location.href",
            ],
            vec![
                "browser-cli",
                "action",
                "eval",
                "--session-id",
                "browser",
                "--expression",
                "location.href",
                "--target-id",
                "result",
            ],
        ] {
            let Command::Action { target_id, .. } = Cli::try_parse_from(args).unwrap().command
            else {
                panic!("expected action");
            };
            assert_eq!(target_id.as_deref(), Some("result"));
        }
    }

    #[test]
    fn existing_action_syntax_has_no_explicit_target() {
        let Command::Action { target_id, .. } = Cli::try_parse_from([
            "browser-cli",
            "action",
            "snapshot",
            "--session-id",
            "browser",
        ])
        .unwrap()
        .command
        else {
            panic!("expected action");
        };
        assert!(target_id.is_none());
    }

    #[test]
    fn page_target_option_is_not_accepted_by_session_commands() {
        assert!(
            Cli::try_parse_from([
                "browser-cli",
                "session",
                "targets",
                "--session-id",
                "browser",
                "--target-id",
                "result"
            ])
            .is_err()
        );
    }

    #[test]
    fn auth_login_defaults_client_name_to_agent() {
        let cli = Cli::try_parse_from(["browser-cli", "auth", "login"]).unwrap();
        let Command::Auth {
            command: AuthCommand::Login { client_name, .. },
        } = cli.command
        else {
            panic!("expected auth login command");
        };

        assert_eq!(client_name, auth::DEFAULT_CLIENT_NAME);
    }

    #[test]
    fn auth_login_parses_custom_client_name() {
        let client_name = "Claude Desktop 中文";
        let cli =
            Cli::try_parse_from(["browser-cli", "auth", "login", "--client-name", client_name])
                .unwrap();
        let Command::Auth {
            command:
                AuthCommand::Login {
                    client_name: parsed,
                    ..
                },
        } = cli.command
        else {
            panic!("expected auth login command");
        };

        assert_eq!(parsed, client_name);
    }
}
