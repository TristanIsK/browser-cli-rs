//! Real process/pipe tests with a gated loopback-only HTTP response.
mod support;

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::TcpListener,
    process::Output,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn closed_reader(fail_action: bool, close_stderr: bool) -> Output {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut length = 0;
        let mut header_bytes = 0;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            header_bytes += line.len();
            assert!(header_bytes < 16384);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        assert!(length < 65536);
        reader.read_exact(&mut vec![0; length]).unwrap();
        ready_tx.send(()).unwrap();
        // Do not let the CLI finish until the test has closed its pipe reader.
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (status, body) = if fail_action {
            (
                "401 Unauthorized",
                json!({"message":"local fixture refused"}),
            )
        } else {
            ("200 OK", json!({"session_id":"fixture","status":"active"}))
        };
        let body = body.to_string();
        write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let mut child = support::command(
        &api,
        directory.path(),
        &["session", "get", "--session-id", "fixture"],
    )
    .spawn()
    .unwrap();
    if ready_rx.recv_timeout(Duration::from_secs(15)).is_err() {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        let _ = server.join();
        panic!(
            "CLI did not reach fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    drop(child.stdout.take());
    if close_stderr {
        drop(child.stderr.take());
    }
    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = server.join();
            panic!("CLI did not exit after pipe closed");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    server.join().unwrap();
    output
}

#[test]
fn closed_stdout_after_success_does_not_panic() {
    let output = closed_reader(false, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn closing_stdout_does_not_hide_an_action_failure() {
    let output = closed_reader(true, false);
    assert_eq!(output.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["ok"], false);
}

#[test]
fn closed_stderr_preserves_failure_exit_without_panic() {
    let output = closed_reader(true, true);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn version_still_emits_a_single_json_document() {
    let directory = tempfile::tempdir().unwrap();
    let output = support::cli("http://127.0.0.1:1", directory.path(), &["version"]);
    assert!(output.stdout.ends_with(b"\n"));
    assert_eq!(output.stdout.iter().filter(|b| **b == b'\n').count(), 1);
    assert_eq!(support::data(output)["version"], env!("CARGO_PKG_VERSION"));
}
