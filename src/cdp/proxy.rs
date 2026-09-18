//! Use the same environment proxy matcher as reqwest for CDP connections.
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use tungstenite::{
    WebSocket,
    handshake::HandshakeError,
    http::{Uri, uri::Scheme},
    stream::MaybeTlsStream,
};

use crate::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    connect_with_matcher(url, &Matcher::from_env())
}

fn destination(url: &str) -> Result<Uri> {
    let uri: Uri = url
        .parse()
        .map_err(|_| Error::Config("invalid CDP URL".into()))?;
    let scheme = match uri.scheme_str() {
        Some("ws") => Scheme::HTTP,
        Some("wss") => Scheme::HTTPS,
        _ => return Err(Error::Config("CDP URL must use ws or wss".into())),
    };
    if uri.host().is_none() {
        return Err(Error::Config("CDP URL has no host".into()));
    }
    let mut parts = uri.into_parts();
    parts.scheme = Some(scheme);
    Uri::from_parts(parts).map_err(|_| Error::Config("invalid CDP URL".into()))
}

fn connect_with_matcher(
    url: &str,
    matcher: &Matcher,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let mut url = url.to_owned();
    // Match tungstenite::connect's three-hop limit, but choose the route anew
    // for every destination, including redirects to/from NO_PROXY hosts.
    for attempt in 0..=3 {
        match connect_once(&url, matcher) {
            Err(Error::WebSocket(error)) if attempt < 3 => {
                if let tungstenite::Error::Http(response) = error.as_ref()
                    && response.status().is_redirection()
                    && let Some(location) = response.headers().get("Location")
                {
                    url = location
                        .to_str()
                        .map_err(|_| Error::Config("invalid CDP redirect Location".into()))?
                        .to_owned();
                    continue;
                }
                return Err(Error::WebSocket(error));
            }
            result => return result,
        }
    }
    unreachable!("last connection attempt always returns")
}

fn connect_once(url: &str, matcher: &Matcher) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let destination = destination(url)?;
    let Some(proxy) = matcher.intercept(&destination) else {
        // The outer loop owns redirects so a direct hop cannot skip proxy rules.
        return Ok(tungstenite::client::connect_with_config(url, None, 0)?.0);
    };
    // ponytail: implement the HTTP CONNECT proxy used by cloud runtimes. Other
    // proxy schemes fail explicitly; never silently retry a proxy failure direct.
    if proxy.uri().scheme_str() != Some("http") {
        return Err(Error::Config(
            "CDP supports only http:// CONNECT proxies".into(),
        ));
    }
    let host = proxy
        .uri()
        .host()
        .ok_or_else(|| Error::Config("proxy has no host".into()))?;
    let port = proxy.uri().port_u16().unwrap_or(80);
    let deadline = Instant::now() + TIMEOUT;
    let mut last_error = std::io::Error::other("proxy has no addresses");
    let mut connected = None;
    for address in (host.trim_matches(['[', ']']), port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, remaining(deadline)?) {
            Ok(stream) => {
                connected = Some(stream);
                break;
            }
            Err(error) => last_error = error,
        }
    }
    let mut stream = connected.ok_or(last_error)?;
    stream.set_write_timeout(Some(remaining(deadline)?))?;
    establish_tunnel(&mut stream, &destination, &proxy, deadline)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    // Keep a handle to restore normal CDP I/O after the TLS/WebSocket handshake.
    let timeout_handle = stream.try_clone()?;
    let (socket, _) = tungstenite::client_tls(url, stream).map_err(|error| match error {
        HandshakeError::Failure(error) => Error::from(error),
        HandshakeError::Interrupted(_) => {
            Error::Cdp("proxy WebSocket handshake interrupted".into())
        }
    })?;
    timeout_handle.set_read_timeout(None)?;
    timeout_handle.set_write_timeout(None)?;
    Ok(socket)
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| Error::Timeout("CDP proxy connection".into()))
}

fn establish_tunnel(
    stream: &mut TcpStream,
    destination: &Uri,
    proxy: &Intercept,
    deadline: Instant,
) -> Result<()> {
    let host = destination
        .host()
        .ok_or_else(|| Error::Config("CDP URL has no host".into()))?;
    let port = destination
        .port_u16()
        .unwrap_or(if destination.scheme_str() == Some("https") {
            443
        } else {
            80
        });
    let authority = format!("{host}:{port}");
    write!(
        stream,
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n"
    )?;
    if let Some(auth) = proxy.basic_auth() {
        stream.write_all(b"Proxy-Authorization: ")?;
        stream.write_all(auth.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.flush()?;
    // Read exactly the CONNECT headers so no TLS/WebSocket bytes are lost.
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 16 * 1024 {
            return Err(Error::Cdp(
                "proxy CONNECT response headers too large".into(),
            ));
        }
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let mut headers = [httparse::EMPTY_HEADER; 128];
    let mut response = httparse::Response::new(&mut headers);
    if !matches!(response.parse(&bytes), Ok(httparse::Status::Complete(_))) {
        return Err(Error::Cdp("invalid proxy CONNECT response".into()));
    }
    match response.code {
        Some(200..=299) => Ok(()),
        Some(code) => Err(Error::Cdp(format!(
            "proxy CONNECT rejected with HTTP {code}"
        ))),
        None => Err(Error::Cdp("proxy CONNECT response has no status".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};
    use tungstenite::{
        Message,
        handshake::server::{Request, Response},
    };

    fn listener() -> TcpListener {
        TcpListener::bind("127.0.0.1:0").unwrap()
    }

    fn read_headers(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
        }
        String::from_utf8(bytes).unwrap()
    }

    fn accept_with_timeout(listener: &TcpListener) -> TcpStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Windows accepted sockets inherit the listener's mode.
                    stream.set_nonblocking(false).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "expected another connection");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    }

    #[test]
    fn redirects_rematch_proxy_and_no_proxy_for_every_hop() {
        let proxy = listener();
        let direct = listener();
        let direct_url = format!("ws://{}/middle", direct.local_addr().unwrap());
        let matcher = Matcher::builder()
            .http(format!("http://{}", proxy.local_addr().unwrap()))
            .no("127.0.0.1")
            .build();
        let proxy_server = thread::spawn(move || {
            for host in ["entry.invalid", "next.invalid", "final.invalid"] {
                let mut stream = accept_with_timeout(&proxy);
                assert!(
                    read_headers(&mut stream)
                        .starts_with(&format!("CONNECT {host}:80 HTTP/1.1\r\n"))
                );
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
                if host != "final.invalid" {
                    read_headers(&mut stream);
                    let location = if host == "entry.invalid" {
                        direct_url.as_str()
                    } else {
                        "ws://final.invalid/cdp"
                    };
                    write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .unwrap();
                } else {
                    let mut socket = tungstenite::accept(stream).unwrap();
                    socket.send(Message::text("redirected")).unwrap();
                }
            }
        });
        let direct_server = thread::spawn(move || {
            let mut stream = accept_with_timeout(&direct);
            let request = read_headers(&mut stream);
            assert!(request.starts_with("GET /middle HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
            stream.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: ws://next.invalid/cdp\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        let mut socket = connect_with_matcher("ws://entry.invalid/cdp", &matcher).unwrap();
        assert_eq!(socket.read().unwrap().into_text().unwrap(), "redirected");
        proxy_server.join().unwrap();
        direct_server.join().unwrap();
    }

    #[test]
    fn redirects_stop_after_three_hops_for_proxy_and_direct_connections() {
        for proxied in [true, false] {
            let endpoint = listener();
            let address = endpoint.local_addr().unwrap();
            let matcher = if proxied {
                Matcher::builder().all(format!("http://{address}")).build()
            } else {
                Matcher::builder().build()
            };
            let url = if proxied {
                "ws://loop.invalid/cdp".to_owned()
            } else {
                format!("ws://{address}/cdp")
            };
            let location = url.clone();
            let server = thread::spawn(move || {
                for _ in 0..4 {
                    let mut stream = accept_with_timeout(&endpoint);
                    if proxied {
                        assert!(read_headers(&mut stream).starts_with("CONNECT "));
                        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
                    }
                    assert!(read_headers(&mut stream).starts_with("GET "));
                    write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .unwrap();
                }
                endpoint
            });
            let error = connect_with_matcher(&url, &matcher).unwrap_err();
            assert!(matches!(error, Error::WebSocket(ref error)
                if matches!(error.as_ref(), tungstenite::Error::Http(response)
                    if response.status().as_u16() == 302)));
            let endpoint = server.join().unwrap();
            assert_eq!(
                endpoint.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn missing_or_invalid_redirect_locations_fail_without_another_connection() {
        for location in [
            "",
            "Location: /relative\r\n",
            "Location: https://browser.invalid/\r\n",
        ] {
            let proxy = listener();
            let matcher = Matcher::builder()
                .all(format!("http://{}", proxy.local_addr().unwrap()))
                .build();
            let server = thread::spawn(move || {
                let mut stream = accept_with_timeout(&proxy);
                read_headers(&mut stream);
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
                read_headers(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 302 Found\r\n{location}Content-Length: 0\r\n\r\n"
                )
                .unwrap();
                proxy
            });
            assert!(connect_with_matcher("ws://browser.invalid/", &matcher).is_err());
            let proxy = server.join().unwrap();
            assert_eq!(
                proxy.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite fixes the callback's response error type"
    )]
    fn connect_uses_proxy_dns_and_keeps_proxy_credentials_out_of_websocket() {
        let proxy = listener();
        let matcher = Matcher::builder()
            .http(format!(
                "http://test-user:test-password@{}",
                proxy.local_addr().unwrap()
            ))
            .build();
        let server = thread::spawn(move || {
            let (mut stream, _) = proxy.accept().unwrap();
            let headers = read_headers(&mut stream);
            assert!(headers.starts_with("CONNECT browser.invalid:80 HTTP/1.1\r\n"));
            assert!(
                headers.contains("Proxy-Authorization: Basic dGVzdC11c2VyOnRlc3QtcGFzc3dvcmQ=\r\n")
            );
            assert!(!headers.contains("private-token"));
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .unwrap();
            let mut ws =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(request.uri(), "/cdp?token=private-token");
                    assert!(!request.headers().contains_key("Proxy-Authorization"));
                    Ok(response)
                })
                .unwrap();
            assert_eq!(ws.read().unwrap().into_text().unwrap(), "probe");
            ws.send(Message::text("ok")).unwrap();
        });
        let mut ws =
            connect_with_matcher("ws://browser.invalid/cdp?token=private-token", &matcher).unwrap();
        ws.send(Message::text("probe")).unwrap();
        assert_eq!(ws.read().unwrap().into_text().unwrap(), "ok");
        server.join().unwrap();
    }

    #[test]
    fn secure_websockets_use_https_proxy_and_no_proxy_is_respected() {
        let matcher = Matcher::builder()
            .http("http://plain-proxy:8080")
            .https("http://secure-proxy:8081")
            .no("localhost,.internal.example,127.0.0.0/8")
            .build();
        let proxy = matcher
            .intercept(&destination("wss://browser.example/cdp").unwrap())
            .unwrap();
        assert_eq!(proxy.uri().host(), Some("secure-proxy"));
        for url in [
            "ws://localhost/cdp",
            "wss://api.internal.example/cdp",
            "ws://127.0.0.1/cdp",
        ] {
            assert!(matcher.intercept(&destination(url).unwrap()).is_none());
        }
        assert!(
            matcher
                .intercept(&destination("wss://notinternal.example/cdp").unwrap())
                .is_some()
        );
    }

    #[test]
    fn proxy_rejection_does_not_retry_direct_or_disclose_response() {
        let proxy = listener();
        let origin = listener();
        origin.set_nonblocking(true).unwrap();
        let matcher = Matcher::builder()
            .all(format!("http://{}", proxy.local_addr().unwrap()))
            .build();
        let server = thread::spawn(move || {
            let (mut stream, _) = proxy.accept().unwrap();
            read_headers(&mut stream);
            stream
                .write_all(b"HTTP/1.1 407 private-secret\r\nX-Secret: hidden\r\n\r\n")
                .unwrap();
        });
        let error = connect_with_matcher(
            &format!("ws://{}/?token=private-token", origin.local_addr().unwrap()),
            &matcher,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "CDP command failed: proxy CONNECT rejected with HTTP 407"
        );
        assert_eq!(
            origin.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        server.join().unwrap();
    }

    #[test]
    fn unsupported_proxy_is_an_error_instead_of_direct_fallback() {
        for scheme in ["socks5", "https"] {
            let matcher = Matcher::builder()
                .all(format!("{scheme}://user:private-secret@proxy.invalid:8080"))
                .build();
            let error =
                connect_with_matcher("wss://browser.invalid/?token=private-token", &matcher)
                    .unwrap_err();
            assert_eq!(
                error.to_string(),
                "configuration error: CDP supports only http:// CONNECT proxies"
            );
        }
    }

    #[test]
    fn tunnel_headers_are_bounded_and_malformed_responses_are_not_echoed() {
        for response in [
            b"invalid private-secret\r\n\r\n".to_vec(),
            vec![b'x'; 16 * 1024],
        ] {
            let proxy = listener();
            let matcher = Matcher::builder()
                .all(format!("http://{}", proxy.local_addr().unwrap()))
                .build();
            let server = thread::spawn(move || {
                let (mut stream, _) = proxy.accept().unwrap();
                read_headers(&mut stream);
                stream.write_all(&response).unwrap();
            });
            let error = connect_with_matcher("ws://browser.invalid/", &matcher)
                .unwrap_err()
                .to_string();
            assert!(error.contains("proxy CONNECT response"));
            assert!(!error.contains("private-secret"));
            server.join().unwrap();
        }
    }
}
