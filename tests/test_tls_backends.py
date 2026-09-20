#!/usr/bin/env python3
"""Downstream SDK feature-unification regression; loopback only, test-only CA."""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import ssl
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
CLIENT = r'''
fn main() {
    let url = std::env::args().nth(1).unwrap();
    let (socket, _) = tungstenite::connect(url.as_str()).expect("control TLS/WS handshake");
    #[cfg(feature = "native-tls")]
    assert!(matches!(socket.get_ref(), tungstenite::stream::MaybeTlsStream::NativeTls(_)));
    #[cfg(not(feature = "native-tls"))]
    assert!(matches!(socket.get_ref(), tungstenite::stream::MaybeTlsStream::Rustls(_)));
    drop(socket);
    let mut cdp = lexmount_browser::cdp::Cdp::connect(&url).expect("SDK connection");
    assert_eq!(cdp.evaluate("1 + 1").unwrap(), 2);
}
'''


def exact(stream, size):
    data = b""
    while len(data) < size:
        part = stream.recv(size - len(data))
        if not part:
            raise EOFError("client closed")
        data += part
    return data


def headers(stream):
    data = b""
    while not data.endswith(b"\r\n\r\n"):
        data += exact(stream, 1)
        assert len(data) < 16384
    return data


def serve(listener, context, errors):
    try:
        for control in (True, False):
            raw, _ = listener.accept()
            with raw:
                raw.settimeout(20)
                if raw.recv(1, socket.MSG_PEEK) == b"C":
                    assert headers(raw).startswith(b"CONNECT ")
                    raw.sendall(b"HTTP/1.1 200 OK\r\n\r\n")
                with context.wrap_socket(raw, server_side=True) as stream:
                    request = headers(stream)
                    key = next(line.split(b":", 1)[1].strip() for line in request.split(b"\r\n")
                               if line.lower().startswith(b"sec-websocket-key:"))
                    accept = base64.b64encode(hashlib.sha1(key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
                    stream.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " + accept + b"\r\n\r\n")
                    if control:
                        continue
                    for method, result in [
                        ("Target.getTargets", {"targetInfos": [{"type": "page", "targetId": "page"}]}),
                        ("Target.attachToTarget", {"sessionId": "attached"}),
                        ("Page.enable", {}), ("Runtime.enable", {}),
                        ("Runtime.evaluate", {"result": {"value": 2}}),
                    ]:
                        opcode, length = exact(stream, 2)
                        assert opcode == 0x81 and length & 0x80
                        length &= 0x7f
                        if length == 126:
                            length = int.from_bytes(exact(stream, 2), "big")
                        assert length < 4096
                        mask = exact(stream, 4)
                        message = json.loads(bytes(value ^ mask[i % 4] for i, value in enumerate(exact(stream, length))))
                        assert message["method"] == method
                        payload = json.dumps({"id": message["id"], "result": result}).encode()
                        assert len(payload) < 126
                        stream.sendall(bytes([0x81, len(payload)]) + payload)
    except Exception as error:
        errors.append(error)


with tempfile.TemporaryDirectory(prefix="browser-cli-tls-") as directory:
    temp = Path(directory)
    (temp / "src").mkdir()
    (temp / "src/main.rs").write_text(CLIENT)
    (temp / "Cargo.toml").write_text(f'''[package]
name = "browser-cli-tls-regression"
version = "0.0.0"
edition = "2024"
[features]
native-tls = ["tungstenite/native-tls"]
[dependencies]
lexmount-browser = {{ path = {json.dumps(str(ROOT))} }}
tungstenite = {{ version = "0.27", default-features = false }}
''')
    # Preserve the SDK's locked versions while adding downstream native-tls deps.
    shutil.copyfile(ROOT / "Cargo.lock", temp / "Cargo.lock")
    for args in [
        ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=Test CA", "-keyout", "ca.key", "-out", "ca.pem", "-addext", "basicConstraints=critical,CA:TRUE"],
        ["req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", "key.pem", "-out", "leaf.csr"],
        ["x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-days", "1", "-out", "cert.pem", "-extfile", "leaf.ext"],
    ]:
        (temp / "leaf.ext").write_text("subjectAltName=IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\n")
        subprocess.run(["openssl", *args], cwd=temp, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(temp / "cert.pem", temp / "key.pem")
    build_env = dict(os.environ)
    build_env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/tls-backends"))
    for backend in ("native-tls", "rustls"):
        command = ["cargo", "build", "--manifest-path", str(temp / "Cargo.toml")]
        if backend == "native-tls":
            command += ["--features", "native-tls"]
        subprocess.run(command, env=build_env, check=True)
        for proxied in (False, True):
            env = {k: v for k, v in build_env.items() if k.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy")}
            env["SSL_CERT_FILE"] = str(temp / "ca.pem")
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen()
                listener.settimeout(20)
                address = f"127.0.0.1:{listener.getsockname()[1]}"
                if proxied:
                    env["HTTPS_PROXY"] = f"http://{address}"
                errors = []
                worker = threading.Thread(target=serve, args=(listener, context, errors), daemon=True)
                worker.start()
                subprocess.run([str(Path(env["CARGO_TARGET_DIR"]) / "debug/browser-cli-tls-regression"), f"wss://{address}/"], env=env, check=True, timeout=25)
                worker.join(timeout=5)
                assert not worker.is_alive() and not errors, errors
                print(f"{backend} {'proxy' if proxied else 'direct'}: TLS verified, SDK connected and evaluated", flush=True)
