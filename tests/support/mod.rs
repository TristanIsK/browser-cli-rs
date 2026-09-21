use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

// Run the real binary against a loopback fixture, never the user's credentials.
pub fn command(api: &str, directory: &Path, arguments: &[&str]) -> Command {
    assert!(api.starts_with("http://127.0.0.1:"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_browser-cli"));
    command
        .args(arguments)
        .current_dir(directory)
        .env("LEXMOUNT_API_KEY", "local-test-key")
        .env("LEXMOUNT_PROJECT_ID", "local-test-project")
        .env("LEXMOUNT_BASE_URL", api)
        .env(
            "LEXMOUNT_BROWSER_CREDENTIALS_FILE",
            directory.join("missing-credentials.json"),
        )
        .env_remove("LEXMOUNT_REGION")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_proxy_env(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

pub fn cli(api: &str, directory: &Path, arguments: &[&str]) -> Output {
    run(command(api, directory, arguments), Duration::from_secs(20))
}

fn clear_proxy_env(command: &mut Command) {
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(name);
    }
}

// Re-run only this SDK test in an isolated process, before starting fixtures.
// No process-global environment mutation or production localhost bypass.
#[allow(dead_code)]
pub fn isolated_test(name: &str, timeout: Duration) -> bool {
    if std::env::var("BROWSER_CLI_ISOLATED_TEST").as_deref() == Ok(name) {
        return false;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env("BROWSER_CLI_ISOLATED_TEST", name)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_proxy_env(&mut command);
    let output = run(command, timeout);
    assert!(
        output.status.success(),
        "isolated test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn run(mut command: Command, timeout: Duration) -> Output {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + timeout;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "fixture CLI timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

pub fn data(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope.as_object().unwrap().len(), 2);
    envelope["data"].clone()
}
