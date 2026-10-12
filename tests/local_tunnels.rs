mod support;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::{
    Method::{DELETE, POST},
    Mock, MockServer,
};
use lexmount_browser::{Client, Error, models::CreateSession};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    Message, WebSocket,
    handshake::server::{Request, Response},
};

const FRAME: usize = 65536;
const WINDOW: usize = 262144;
fn id() -> String {
    format!(
        "lt1.{}.12345678-1234-1234-1234-123456789abc",
        URL_SAFE_NO_PAD.encode("nanjing-1")
    )
}
fn client(api: &MockServer) -> Client {
    Client::builder()
        .api_key("local-test-key")
        .project_id("local-test-project")
        .base_url(api.base_url())
        .build()
        .unwrap()
}
fn setup<'a>(api: &'a MockServer, ws: &str) -> (Mock<'a>, Mock<'a>) {
    let create = api.mock(|when, then| { when.method(POST).path("/instance/v1/tunnels")
        .header("x-api-key", "local-test-key").header("x-project-id", "local-test-project");
        then.status(201).json_body(json!({"id":id(),"region_id":"nanjing-1","ticket":"single-use-test-ticket","ws_url":ws})); });
    let close = api.mock(|when, then| {
        when.method(DELETE)
            .path(format!("/instance/v1/tunnels/{}", id()))
            .json_body(json!({"region_id":"nanjing-1"}));
        then.status(200).json_body(json!({"closed":true}));
    });
    (create, close)
}
#[allow(clippy::result_large_err)] // tungstenite requires this callback error type.
fn ws_accept(listener: &TcpListener, token: &str) -> WebSocket<TcpStream> {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    tungstenite::accept_hdr(stream, |req: &Request, mut res: Response| {
        assert_eq!(req.headers()["authorization"], format!("Bearer {token}"));
        assert_eq!(
            req.headers()["sec-websocket-protocol"],
            "lexmount.local-proxy.v1"
        );
        assert!(!req.uri().to_string().contains("ticket"));
        res.headers_mut().insert(
            "sec-websocket-protocol",
            "lexmount.local-proxy.v1".parse().unwrap(),
        );
        Ok(res)
    })
    .unwrap()
}
fn control(ws: &mut WebSocket<TcpStream>, value: Value) {
    ws.send(Message::Text(value.to_string().into())).unwrap();
}
fn read_control(ws: &mut WebSocket<TcpStream>) -> Value {
    loop {
        match ws.read().unwrap() {
            Message::Text(s) => return serde_json::from_str(&s).unwrap(),
            Message::Pong(_) => (),
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}
fn ready(ws: &mut WebSocket<TcpStream>, epoch: &str) {
    control(ws, json!({"type":"ready","id":id(),"epoch":epoch}));
}
fn wait_closed(ws: &mut WebSocket<TcpStream>) {
    loop {
        match ws.read() {
            Err(_) | Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => {
                let _ = ws.flush();
            }
            _ => (),
        }
    }
}
fn wait_until(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !check() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn large_duplex_transfer_half_close_and_drop_revocation() {
    if support::isolated_test(
        "large_duplex_transfer_half_close_and_drop_revocation",
        Duration::from_secs(30),
    ) {
        return;
    }
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = target.local_addr().unwrap().port();
    let payload: Vec<_> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let expected = payload.clone();
    let local = thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        bytes.reverse();
        stream.write_all(&bytes).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "ws://{}/local-proxy/{}",
        listener.local_addr().unwrap(),
        id()
    );
    let (done, completed) = mpsc::channel();
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        control(
            &mut ws,
            json!({"type":"open","id":1,"host":"localhost","port":port}),
        );
        assert_eq!(read_control(&mut ws), json!({"type":"opened","id":1}));
        let mut sent = 0;
        let mut credit = WINDOW;
        while sent < payload.len() {
            while credit > 0 && sent < payload.len() {
                let n = FRAME.min(credit).min(payload.len() - sent);
                let mut frame = 1u32.to_be_bytes().to_vec();
                frame.extend_from_slice(&payload[sent..sent + n]);
                ws.send(Message::Binary(frame.into())).unwrap();
                sent += n;
                credit -= n;
            }
            if sent < payload.len() {
                let ack = read_control(&mut ws);
                assert_eq!(ack["type"], "window");
                credit += ack["credit"].as_u64().unwrap() as usize;
            }
        }
        control(&mut ws, json!({"type":"fin","id":1}));
        let mut received = Vec::new();
        loop {
            match ws.read().unwrap() {
                Message::Binary(data) => {
                    assert_eq!(&data[..4], &1u32.to_be_bytes());
                    received.extend_from_slice(&data[4..]);
                    control(
                        &mut ws,
                        json!({"type":"window","id":1,"credit":data.len()-4}),
                    );
                }
                Message::Text(s) => {
                    let value: Value = serde_json::from_str(&s).unwrap();
                    if value["type"] == "fin" {
                        break;
                    }
                    assert_eq!(value["type"], "window");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(received, payload.into_iter().rev().collect::<Vec<_>>());
        // Unknown/reset streams may have data already in flight.
        ws.send(Message::Binary(vec![0, 0, 0, 1, 5].into()))
            .unwrap();
        done.send(()).unwrap();
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (create, close) = setup(&api, &url);
    let tunnel = client(&api).open_tunnel().unwrap();
    assert!(tunnel.ready());
    assert_eq!(tunnel.region_id(), "nanjing-1");
    completed.recv_timeout(Duration::from_secs(15)).unwrap();
    drop(tunnel);
    create.assert_hits(1);
    close.assert_hits(1);
    gateway.join().unwrap();
    local.join().unwrap();
}

#[test]
fn reconnect_uses_new_ticket_and_closes_old_tcp_streams() {
    if support::isolated_test(
        "reconnect_uses_new_ticket_and_closes_old_tcp_streams",
        Duration::from_secs(20),
    ) {
        return;
    }
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = target.local_addr().unwrap().port();
    let local = thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
    let (done, completed) = mpsc::channel();
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        control(
            &mut ws,
            json!({"type":"open","id":1,"host":"localhost","port":port}),
        );
        assert_eq!(read_control(&mut ws)["type"], "opened");
        drop(ws);
        let mut ws = ws_accept(&listener, "replacement-ticket");
        ready(&mut ws, "two");
        // New epoch may reuse IDs, and ping/pong stays on the WebSocket.
        ws.send(Message::Ping(vec![1, 2, 3].into())).unwrap();
        assert_eq!(ws.read().unwrap(), Message::Pong(vec![1, 2, 3].into()));
        done.send(()).unwrap();
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (_, close) = setup(&api, &url);
    let ticket = api.mock(|when, then| {
        when.method(POST)
            .path(format!("/instance/v1/tunnels/{}/connect-ticket", id()))
            .json_body(json!({"region_id":"nanjing-1"}));
        then.status(200)
            .json_body(json!({"ticket":"replacement-ticket"}));
    });
    let mut tunnel = client(&api).open_tunnel().unwrap();
    completed.recv_timeout(Duration::from_secs(8)).unwrap();
    assert!(tunnel.ready());
    tunnel.close().unwrap();
    ticket.assert_hits(1);
    close.assert_hits(1);
    gateway.join().unwrap();
    local.join().unwrap();
}

#[test]
fn invalid_credit_disconnects_and_revoked_tunnel_stops_reconnecting() {
    if support::isolated_test(
        "invalid_credit_disconnects_and_revoked_tunnel_stops_reconnecting",
        Duration::from_secs(15),
    ) {
        return;
    }
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = target.local_addr().unwrap().port();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        control(
            &mut ws,
            json!({"type":"open","id":1,"host":"127.0.0.1","port":port}),
        );
        assert_eq!(read_control(&mut ws)["type"], "opened");
        control(&mut ws, json!({"type":"window","id":1,"credit":1}));
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (_, close) = setup(&api, &url);
    let ticket = api.mock(|when, then| {
        when.method(POST)
            .path(format!("/instance/v1/tunnels/{}/connect-ticket", id()));
        then.status(404).json_body(json!({"error":"closed"}));
    });
    let mut tunnel = client(&api).open_tunnel().unwrap();
    wait_until(|| !tunnel.is_running());
    assert!(!tunnel.ready());
    tunnel.close().unwrap();
    ticket.assert_hits(1);
    close.assert_hits(1);
    gateway.join().unwrap();
}

#[test]
fn bad_ready_and_redirect_fail_closed_and_revoke() {
    if support::isolated_test(
        "bad_ready_and_redirect_fail_closed_and_revoke",
        Duration::from_secs(15),
    ) {
        return;
    }
    for redirect in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        destination.set_nonblocking(true).unwrap();
        let address = destination.local_addr().unwrap();
        let gateway = thread::spawn(move || {
            if redirect {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(stream,"HTTP/1.1 302 Found\r\nLocation: ws://{address}/stolen\r\nContent-Length: 0\r\n\r\n").unwrap();
            } else {
                let mut ws = ws_accept(&listener, "single-use-test-ticket");
                control(&mut ws, json!({"type":"ready","id":"wrong","epoch":"one"}));
                wait_closed(&mut ws);
            }
        });
        let api = MockServer::start();
        let (create, close) = setup(&api, &url);
        let error = client(&api).open_tunnel().err().unwrap().to_string();
        assert!(!error.contains("single-use-test-ticket"));
        create.assert_hits(1);
        close.assert_hits(1);
        gateway.join().unwrap();
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn local_session_region_binding_and_cli_conflicts() {
    if support::isolated_test(
        "local_session_region_binding_and_cli_conflicts",
        Duration::from_secs(15),
    ) {
        return;
    }
    let api = MockServer::start();
    let create = api.mock(|when, then| {
        when.method(POST).path("/instance/v2").json_body_partial(
            json!({"region_id":"nanjing-1","proxy":{"type":"local","tunnel_id":id()}}).to_string(),
        );
        then.status(202)
            .json_body(json!({"session_id":"session-test"}));
    });
    let get=api.mock(|when,then|{when.method(POST).path("/instance/session");then.status(200).json_body(json!({"session_id":"session-test","status":"active","ws":"ws://127.0.0.1:1","region_id":"nanjing-1"}));});
    let temp = tempfile::tempdir().unwrap();
    let data = support::data(support::cli(
        &api.base_url(),
        temp.path(),
        &["session", "create", "--local-proxy-tunnel-id", &id()],
    ));
    assert_eq!(data["session_id"], "session-test");
    create.assert_hits(1);
    get.assert_hits(1);
    let output = support::cli(
        &api.base_url(),
        temp.path(),
        &[
            "session",
            "create",
            "--local-proxy-tunnel-id",
            &id(),
            "--official-proxy",
        ],
    );
    assert!(!output.status.success());
    create.assert_hits(1);
    let client = client(&api);
    let request = CreateSession {
        proxy: Some(json!({"type":"local","tunnel_id":id()})),
        official_proxy: Some(true),
        ..Default::default()
    };
    assert!(matches!(
        client.create_session(request, Duration::from_secs(1)),
        Err(Error::Config(_))
    ));
    let other = Client::builder()
        .api_key("x")
        .project_id("x")
        .base_url(api.base_url())
        .region("hk-1")
        .build()
        .unwrap();
    assert!(matches!(other.tunnel_status(&id()), Err(Error::Config(_))));
    create.assert_hits(1);
}

#[test]
fn foreground_cli_prints_ready_once_then_revokes_at_duration() {
    if support::isolated_test(
        "foreground_cli_prints_ready_once_then_revokes_at_duration",
        Duration::from_secs(15),
    ) {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (_, close) = setup(&api, &url);
    let temp = tempfile::tempdir().unwrap();
    let mut child = support::command(
        &api.base_url(),
        temp.path(),
        &["tunnel", "open", "--duration", "1"],
    )
    .spawn()
    .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["data"]["ready"], true);
    assert_eq!(ready["data"]["tunnel_id"], id());
    assert!(!line.contains("ticket"));
    assert!(child.try_wait().unwrap().is_none());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    assert!(rest.is_empty());
    close.assert_hits(1);
    gateway.join().unwrap();
}

#[test]
fn stream_failures_are_isolated_and_reset_closes_local_socket() {
    if support::isolated_test(
        "stream_failures_are_isolated_and_reset_closes_local_socket",
        Duration::from_secs(20),
    ) {
        return;
    }
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = target.local_addr().unwrap().port();
    let local = thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(b"still-alive").unwrap();
        let mut b = [0];
        assert_eq!(stream.read(&mut b).unwrap(), 0);
    });
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let bad_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
    let (done, completed) = mpsc::channel();
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        control(
            &mut ws,
            json!({"type":"open","id":1,"host":"127.0.0.1","port":bad_port}),
        );
        control(
            &mut ws,
            json!({"type":"open","id":2,"host":"127.0.0.1","port":port}),
        );
        let mut failed = false;
        let mut opened = false;
        let mut data = Vec::new();
        while !failed || !opened || data.len() < 11 {
            match ws.read().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    match value["type"].as_str().unwrap() {
                        "reset" => {
                            assert_eq!(value["id"], 1);
                            failed = true;
                        }
                        "opened" => {
                            assert_eq!(value["id"], 2);
                            opened = true;
                        }
                        other => panic!("unexpected {other}"),
                    }
                }
                Message::Binary(bytes) => {
                    assert_eq!(&bytes[..4], &2u32.to_be_bytes());
                    data.extend_from_slice(&bytes[4..]);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(data, b"still-alive");
        control(&mut ws, json!({"type":"reset","id":2,"code":"closed"}));
        // Late data/FIN/window for a reset stream must not revive it.
        ws.send(Message::Binary(vec![0, 0, 0, 2, 1].into()))
            .unwrap();
        control(&mut ws, json!({"type":"fin","id":2}));
        done.send(()).unwrap();
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (_, close) = setup(&api, &url);
    let mut tunnel = client(&api).open_tunnel().unwrap();
    completed.recv_timeout(Duration::from_secs(8)).unwrap();
    local.join().unwrap();
    assert!(tunnel.ready());
    tunnel.close().unwrap();
    gateway.join().unwrap();
    close.assert_hits(1);
}

#[cfg(unix)]
#[test]
fn sigterm_revokes_foreground_tunnel() {
    if support::isolated_test("sigterm_revokes_foreground_tunnel", Duration::from_secs(15)) {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/local-proxy", listener.local_addr().unwrap());
    let gateway = thread::spawn(move || {
        let mut ws = ws_accept(&listener, "single-use-test-ticket");
        ready(&mut ws, "one");
        wait_closed(&mut ws);
    });
    let api = MockServer::start();
    let (_, close) = setup(&api, &url);
    let temp = tempfile::tempdir().unwrap();
    let mut child = support::command(&api.base_url(), temp.path(), &["tunnel", "open"])
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("\"ready\":true"));
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    wait_until(|| child.try_wait().unwrap().is_some());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    close.assert_hits(1);
    gateway.join().unwrap();
}
