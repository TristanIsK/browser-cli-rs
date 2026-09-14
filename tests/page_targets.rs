//! Deterministic protocol and CLI tests. Only local HTTP/WebSocket fixtures run.
mod support;

use httpmock::{Method::POST, MockServer};
use lexmount_browser::cdp::Cdp;
use serde_json::{Value, json};
use std::{
    fs,
    io::ErrorKind,
    net::TcpListener,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tungstenite::{Message, error::ProtocolError};

struct Peer {
    url: String,
    worker: JoinHandle<Vec<Value>>,
}

impl Peer {
    fn start(targets: Value, fail_attach: bool) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                }
            };
            // Windows inherits the listener's nonblocking mode on accept.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut seen = Vec::new();
            let mut attached = String::new();
            loop {
                let raw = match socket.read() {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(
                        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed,
                    )
                    | Err(tungstenite::Error::Protocol(
                        ProtocolError::ResetWithoutClosingHandshake,
                    )) => break,
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
                        ) =>
                    {
                        break;
                    }
                    Err(e) => panic!("fixture read: {e}"),
                };
                let request: Value = serde_json::from_str(&raw).unwrap();
                seen.push(request.clone());
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "Target.getTargets" => json!({"targetInfos":targets}),
                    "Target.createTarget" => {
                        assert_eq!(request["params"], json!({"url":"about:blank"}));
                        json!({"targetId":"created"})
                    }
                    "Target.attachToTarget" => {
                        assert_eq!(request["params"]["flatten"], true);
                        if fail_attach {
                            socket
                                .send(Message::Text(
                                    json!({"id":request["id"],"error":{
                                        "code":-32602,"message":"No target with given id found"
                                    }})
                                    .to_string()
                                    .into(),
                                ))
                                .unwrap();
                            continue;
                        }
                        attached = request["params"]["targetId"].as_str().unwrap().to_owned();
                        json!({"sessionId":format!("attached-{attached}")})
                    }
                    _ => {
                        assert!(!attached.is_empty());
                        assert_eq!(request["sessionId"], format!("attached-{attached}"));
                        match method {
                            "Page.enable" | "Runtime.enable" => json!({}),
                            "Page.navigate" => json!({"frameId":"frame"}),
                            "Page.getFrameTree" => json!({"selectedTarget":attached}),
                            "Page.getLayoutMetrics" => {
                                json!({"cssContentSize":{"width":10,"height":10}})
                            }
                            "Page.captureScreenshot" | "Page.printToPDF" => {
                                json!({"data":"YXJ0aWZhY3Q="})
                            }
                            "Runtime.evaluate" => {
                                let expression = request["params"]["expression"].as_str().unwrap();
                                let value = match expression {
                                    "document.readyState" => json!("complete"),
                                    "location.href" => {
                                        json!(format!("https://example.test/{attached}"))
                                    }
                                    "document.title" => json!(attached),
                                    value if value.starts_with("(()=>({url:") => {
                                        json!({"url":format!("https://example.test/{attached}")})
                                    }
                                    value if value.contains("const visible=") => {
                                        json!([{"text":"Saved"}])
                                    }
                                    _ => json!(true),
                                };
                                json!({"result":{"value":value}})
                            }
                            _ => panic!("unexpected method: {method}"),
                        }
                    }
                };
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
            }
            seen
        });
        Self { url, worker }
    }

    fn finish(self) -> Vec<Value> {
        self.worker.join().unwrap()
    }
}

fn api(websocket: &str) -> MockServer {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST)
            .path("/instance/session")
            .json_body_partial(r#"{"session_id":"browser"}"#);
        then.status(200)
            .json_body(json!({"session_id":"browser","status":"active","ws":websocket}));
    });
    server
}

fn two_pages() -> Value {
    json!([
        {"targetId":"home","type":"page"},
        {"targetId":"wanted","type":"page"},
        {"targetId":"worker","type":"service_worker"}
    ])
}

#[test]
fn sdk_selects_the_requested_target_regardless_of_enumeration_order() {
    for reversed in [false, true] {
        let mut targets = two_pages();
        if reversed {
            targets.as_array_mut().unwrap().reverse();
        }
        let peer = Peer::start(targets, false);
        {
            let mut cdp = Cdp::connect_to_target(&peer.url, "wanted").unwrap();
            assert_eq!(
                cdp.evaluate("location.href").unwrap(),
                "https://example.test/wanted"
            );
        }
        let seen = peer.finish();
        assert_eq!(seen[1]["params"]["targetId"], "wanted");
        assert!(seen.iter().all(|v| v["method"] != "Target.createTarget"));
    }
}

#[test]
fn every_cli_action_routes_to_the_explicit_target_and_preserves_json_output() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["open-url", "--url", "https://example.test/wanted"],
        vec!["eval", "--expression", "location.href"],
        vec!["wait-selector", "--selector", "#ready"],
        vec!["wait-text", "--text", "Saved"],
        vec!["click", "--selector", "#button"],
        vec!["fill", "--selector", "#input", "--value", "hello"],
        vec!["screenshot", "--path", "image.png", "--full-page"],
        vec!["pdf", "--path", "page.pdf", "--print-background"],
        vec!["snapshot"],
        vec!["raw", "--method", "Page.getFrameTree"],
    ] {
        let peer = Peer::start(two_pages(), false);
        let server = api(&peer.url);
        let mut args = vec!["action"];
        args.extend(&arguments);
        args.extend(["--session-id", "browser", "--target-id", "wanted"]);
        let result = support::data(support::cli(&server.base_url(), directory.path(), &args));
        match arguments[0] {
            "open-url" | "snapshot" => assert_eq!(result["url"], "https://example.test/wanted"),
            "eval" => assert_eq!(result, "https://example.test/wanted"),
            "wait-selector" | "wait-text" => assert_eq!(result["found"], true),
            "click" | "fill" => assert_eq!(result, true),
            "screenshot" | "pdf" => {
                assert_eq!(result["bytes"], 8);
                assert_eq!(
                    fs::read(directory.path().join(arguments[2])).unwrap(),
                    b"artifact"
                );
            }
            "raw" => assert_eq!(result["selectedTarget"], "wanted"),
            _ => unreachable!(),
        }
        let seen = peer.finish();
        assert_eq!(seen[1]["params"]["targetId"], "wanted");
        assert!(seen.iter().all(|v| v["method"] != "Target.createTarget"));
        assert!(
            seen.iter()
                .skip(2)
                .all(|v| v["sessionId"] == "attached-wanted")
        );
    }
}

#[test]
fn invalid_cli_targets_fail_without_attaching_creating_or_overwriting_files() {
    for (targets, id, error) in [
        (two_pages(), "closed-or-other-session", "not_found"),
        (json!([]), "missing", "not_found"),
        (two_pages(), "worker", "configuration_error"),
    ] {
        let peer = Peer::start(targets, false);
        let server = api(&peer.url);
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("existing.png"), b"do not replace").unwrap();
        let output = support::cli(
            &server.base_url(),
            directory.path(),
            &[
                "action",
                "screenshot",
                "--session-id",
                "browser",
                "--target-id",
                id,
                "--path",
                "existing.png",
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let result: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"], error);
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("session targets")
        );
        assert_eq!(
            fs::read(directory.path().join("existing.png")).unwrap(),
            b"do not replace"
        );
        let seen = peer.finish();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["method"], "Target.getTargets");
    }
}

#[test]
fn target_closed_between_listing_and_attach_fails_without_fallback() {
    let peer = Peer::start(two_pages(), true);
    let server = api(&peer.url);
    let directory = tempfile::tempdir().unwrap();
    let output = support::cli(
        &server.base_url(),
        directory.path(),
        &[
            "action",
            "click",
            "--session-id",
            "browser",
            "--target-id",
            "wanted",
            "--selector",
            "#button",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(result["error"], "cdp_error");
    let seen = peer.finish();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1]["method"], "Target.attachToTarget");
    assert_eq!(seen[1]["params"]["targetId"], "wanted");
}

#[test]
fn default_cli_still_uses_first_page_or_creates_blank_when_no_pages_exist() {
    for (targets, selected, creates) in [(two_pages(), "home", false), (json!([]), "created", true)]
    {
        let peer = Peer::start(targets, false);
        let server = api(&peer.url);
        let directory = tempfile::tempdir().unwrap();
        let result = support::data(support::cli(
            &server.base_url(),
            directory.path(),
            &[
                "action",
                "eval",
                "--session-id",
                "browser",
                "--expression",
                "location.href",
            ],
        ));
        assert_eq!(result, format!("https://example.test/{selected}"));
        let seen = peer.finish();
        assert_eq!(
            seen.iter().any(|v| v["method"] == "Target.createTarget"),
            creates
        );
        let attached = seen
            .iter()
            .find(|v| v["method"] == "Target.attachToTarget")
            .unwrap();
        assert_eq!(attached["params"]["targetId"], selected);
    }
}
