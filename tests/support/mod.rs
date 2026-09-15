use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

// Run the real binary against a loopback fixture, never the user's credentials.
pub fn cli(api: &str, directory: &Path, arguments: &[&str]) -> Output {
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
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("LEXMOUNT_REGION")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
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
