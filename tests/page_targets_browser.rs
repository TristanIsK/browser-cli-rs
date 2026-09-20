//! Opt-in end-to-end regression with a separate headless Chrome profile.
//! No GUI, cloud session, real site, or user credentials are used.
mod support;

use httpmock::{Method::GET, Method::POST, MockServer};
use lexmount_browser::cdp::Cdp;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const HOME: &str = r#"<!doctype html><title>Search fixture</title>
<input id="query"><button id="search">Search</button>
<script>document.querySelector('#search').onclick = () => {
  window.open('/result?q=' + encodeURIComponent(document.querySelector('#query').value), '_blank');
};</script>"#;

const RESULT: &str = r#"<!doctype html><title>Result fixture</title>
<p id="result">Search result</p><input id="query"><button id="save">Save</button>
<script>document.querySelector('#save').onclick = () => {
  document.querySelector('#result').textContent = document.querySelector('#query').value;
};</script>"#;

struct Browser {
    process: Child,
    _profile: tempfile::TempDir,
    websocket: String,
    http: String,
}

impl Browser {
    fn start(executable: &Path) -> Self {
        let profile = tempfile::Builder::new()
            .prefix("browser-cli-targets-")
            .tempdir()
            .unwrap();
        let mut command = Command::new(executable);
        command.args([
            "--headless",
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1",
            "--no-proxy-server",
        ]);
        command.arg(format!("--user-data-dir={}", profile.path().display()));
        command.arg("about:blank");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let process = command.spawn().unwrap();
        let mut browser = Self {
            process,
            _profile: profile,
            websocket: String::new(),
            http: String::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(port_file) =
                fs::read_to_string(browser._profile.path().join("DevToolsActivePort"))
            {
                let mut lines = port_file.lines();
                let port: u16 = lines.next().unwrap().parse().unwrap();
                let path = lines.next().unwrap();
                assert!(path.starts_with("/devtools/browser/"));
                browser.websocket = format!("ws://127.0.0.1:{port}{path}");
                browser.http = format!("http://127.0.0.1:{port}");
                return browser;
            }
            assert!(
                browser.process.try_wait().unwrap().is_none(),
                "headless browser exited"
            );
            assert!(Instant::now() < deadline, "headless startup timed out");
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Close only this test's isolated browser, including its renderer children.
        if !self.websocket.is_empty()
            && let Ok((mut socket, _)) = tungstenite::connect(self.websocket.as_str())
        {
            let _ = socket.send(tungstenite::Message::Text(
                json!({"id":1,"method":"Browser.close"}).to_string().into(),
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.process.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn pages(cdp: &mut Cdp) -> Vec<Value> {
    cdp.command_root("Target.getTargets", json!({})).unwrap()["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["type"] == "page")
        .cloned()
        .collect()
}

#[test]
#[ignore = "requires BROWSER_CLI_TEST_CHROME pointing to a Chrome/Chromium executable"]
fn search_popup_can_be_selected_across_cli_invocations_and_closed_safely() {
    if support::isolated_test(
        "search_popup_can_be_selected_across_cli_invocations_and_closed_safely",
        Duration::from_secs(120),
    ) {
        return;
    }
    let chromium = std::env::var_os("BROWSER_CLI_TEST_CHROME")
        .expect("set BROWSER_CLI_TEST_CHROME to a local Chrome/Chromium executable");
    let browser = Browser::start(Path::new(&chromium));
    let directory = tempfile::tempdir().unwrap();
    let fixture = MockServer::start();
    fixture.mock(|when, then| {
        when.method(GET).path("/home");
        then.status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(HOME);
    });
    fixture.mock(|when, then| {
        when.method(GET).path("/result");
        then.status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(RESULT);
    });
    let api = MockServer::start();
    api.mock(|when, then| {
        when.method(POST).path("/instance/session");
        then.status(200)
            .json_body(json!({"session_id":"browser","status":"active","ws":browser.websocket}));
    });
    let run = |args: &[&str]| support::data(support::cli(&api.base_url(), directory.path(), args));
    let mut observer = Cdp::connect(&browser.websocket).unwrap();
    let version = observer
        .command_root("Browser.getVersion", json!({}))
        .unwrap();
    let initial = pages(&mut observer);
    assert_eq!(initial.len(), 1);
    let home_id = initial[0]["targetId"].as_str().unwrap();
    let home_url = fixture.url("/home");
    // Legacy syntax still works for the initial single page.
    assert_eq!(
        run(&[
            "action",
            "open-url",
            "--session-id",
            "browser",
            "--url",
            &home_url
        ])["url"],
        home_url
    );
    run(&[
        "action",
        "wait-selector",
        "--session-id",
        "browser",
        "--target-id",
        home_id,
        "--selector",
        "#query",
    ]);
    run(&[
        "action",
        "fill",
        "--session-id",
        "browser",
        "--target-id",
        home_id,
        "--selector",
        "#query",
        "--value",
        "page target",
    ]);
    run(&[
        "action",
        "click",
        "--session-id",
        "browser",
        "--target-id",
        home_id,
        "--selector",
        "#search",
    ]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        let targets = pages(&mut observer);
        if let Some(result) = targets.iter().find(|v| {
            v["url"]
                .as_str()
                .is_some_and(|url| url.starts_with(&fixture.url("/result")))
        }) {
            break result.clone();
        }
        assert!(Instant::now() < deadline, "search popup did not open");
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(result["openerId"], home_id);
    let result_id = result["targetId"].as_str().unwrap();
    let result_url = result["url"].as_str().unwrap();

    // Reuse the real browser's /json listing as the local session-targets API.
    // This proves its page `id` is usable as --target-id, not a CDP sessionId.
    let listed: Value = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(format!("{}/json", browser.http))
        .send()
        .unwrap()
        .json()
        .unwrap();
    api.mock(|when, then| {
        when.method(GET)
            .path("/json")
            .query_param("session_id", "browser");
        then.status(200).json_body(listed.clone());
    });
    let discovered = run(&["session", "targets", "--session-id", "browser"]);
    let selected = discovered
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "page" && v["url"] == result_url)
        .unwrap();
    assert_eq!(selected["id"], result_id);
    let selected_id = selected["id"].as_str().unwrap();
    run(&[
        "action",
        "wait-selector",
        "--session-id",
        "browser",
        "--target-id",
        selected_id,
        "--selector",
        "#result",
    ]);
    for _ in 0..3 {
        assert_eq!(
            run(&[
                "action",
                "eval",
                "--session-id",
                "browser",
                "--target-id",
                selected_id,
                "--expression",
                "location.href"
            ]),
            result_url
        );
    }
    // An unrelated tab's lifecycle cannot change explicit action routing.
    let extra = observer
        .command_root("Target.createTarget", json!({"url":"about:blank"}))
        .unwrap();
    observer
        .command_root("Target.closeTarget", json!({"targetId":extra["targetId"]}))
        .unwrap();
    run(&[
        "action",
        "fill",
        "--session-id",
        "browser",
        "--target-id",
        selected_id,
        "--selector",
        "#query",
        "--value",
        "saved on result",
    ]);
    run(&[
        "action",
        "click",
        "--session-id",
        "browser",
        "--target-id",
        selected_id,
        "--selector",
        "#save",
    ]);
    let snapshot = run(&[
        "action",
        "snapshot",
        "--session-id",
        "browser",
        "--target-id",
        selected_id,
    ]);
    assert_eq!(snapshot["url"], result_url);
    assert!(
        snapshot["text"]
            .as_str()
            .unwrap()
            .contains("saved on result")
    );
    assert_eq!(
        observer
            .evaluate("document.querySelector('#query').value")
            .unwrap(),
        "page target"
    );
    assert_eq!(observer.evaluate("location.href").unwrap(), home_url);

    observer
        .command_root("Target.closeTarget", json!({"targetId":selected_id}))
        .unwrap();
    let output = support::cli(
        &api.base_url(),
        directory.path(),
        &[
            "action",
            "fill",
            "--session-id",
            "browser",
            "--target-id",
            selected_id,
            "--selector",
            "#query",
            "--value",
            "must not reach home",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"], "not_found");
    assert_eq!(
        observer
            .evaluate("document.querySelector('#query').value")
            .unwrap(),
        "page target"
    );
    let remaining = pages(&mut observer);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["targetId"], home_id);
    println!(
        "{}",
        json!({"browser":version["product"],"protocol":version["protocolVersion"],
        "legacy_single_page":"passed","popup_discovery":"passed","explicit_reconnects":3,
        "unrelated_tab_lifecycle":"passed","result_page_mutation":"passed",
        "closed_target_no_fallback":"passed","source_page_unchanged":"passed"})
    );
}
