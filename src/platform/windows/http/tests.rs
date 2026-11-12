use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Instant,
};

fn listener() -> (TcpListener, String) {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/test", server.local_addr().unwrap());
    (server, url)
}

fn headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() < 8192);
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn warm_get_and_streamed_upload_reuse_the_same_tcp_connection() {
    let (server, url) = listener();
    let length = 512 * 1024 + 17;
    let serving = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        assert!(headers(&mut stream).starts_with("GET "));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        let received = headers(&mut stream).to_lowercase();
        assert!(received.starts_with("post "));
        assert!(received.contains(&format!("content-length: {length}\r\n")));
        assert!(!received.contains("transfer-encoding"));
        let mut count = 0;
        let mut buffer = [0; 8192];
        while count < length {
            let limit = buffer.len().min(length - count);
            let n = stream.read(&mut buffer[..limit]).unwrap();
            assert!(n != 0 && buffer[..n].iter().all(|b| *b == 42));
            count += n;
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    let mut client = Client::new().unwrap();
    {
        let mut warm = client
            .request(
                "GET",
                &url,
                "",
                Duration::from_secs(1),
                Duration::from_secs(3),
                0,
            )
            .unwrap();
        warm.send(std::io::empty(), 0).unwrap();
        assert_eq!(warm.status().unwrap(), 200);
        assert_eq!(std::io::copy(&mut warm, &mut std::io::sink()).unwrap(), 2);
    }
    let connection = client.connection.as_ref().unwrap().2 .0;
    let mut upload = client
        .request(
            "POST",
            &url,
            "",
            Duration::from_secs(1),
            Duration::from_secs(3),
            0,
        )
        .unwrap();
    upload.send(std::io::repeat(42), length as u64).unwrap();
    assert_eq!(upload.status().unwrap(), 200);
    drop(upload);
    assert_eq!(client.connection.as_ref().unwrap().2 .0, connection);
    serving.join().unwrap();
}

#[test]
fn slow_drip_body_obeys_the_total_deadline_and_closes_pending_reads() {
    let (server, url) = listener();
    let serving = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        headers(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .unwrap();
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(30));
            if stream.write_all(b"x").is_err() {
                break;
            }
        }
    });
    let mut client = Client::new().unwrap();
    let start = Instant::now();
    let mut request = client
        .request(
            "GET",
            &url,
            "",
            Duration::from_secs(1),
            Duration::from_millis(150),
            0,
        )
        .unwrap();
    request.send(std::io::empty(), 0).unwrap();
    let error = request.read_to_end(&mut Vec::new()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(request);
    assert!(start.elapsed() < Duration::from_millis(500));
    serving.join().unwrap();
}

#[test]
fn basic_only_proxy_is_refused_without_sending_credentials() {
    let (server, url) = listener();
    let serving = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let received = headers(&mut stream).to_lowercase();
        assert!(!received.contains("authorization:"));
        stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let mut client = Client::new().unwrap();
    let mut request = client
        .request(
            "GET",
            &url,
            "",
            Duration::from_secs(1),
            Duration::from_secs(3),
            0,
        )
        .unwrap();
    request.send(std::io::empty(), 0).unwrap();
    assert_eq!(request.status().unwrap(), 407);
    serving.join().unwrap();
}

#[test]
fn redirects_are_optional_and_truncated_uploads_fail() {
    for redirects in [0, 5] {
        let (server, url) = listener();
        let destination = url.replace("/test", "/destination");
        let serving = thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            headers(&mut stream);
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            if redirects != 0 {
                let (mut stream, _) = server.accept().unwrap();
                assert!(headers(&mut stream).starts_with("GET /destination "));
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            }
        });
        let mut client = Client::new().unwrap();
        let mut request = client
            .request(
                "GET",
                &url,
                "",
                Duration::from_secs(1),
                Duration::from_secs(3),
                redirects,
            )
            .unwrap();
        request.send(std::io::empty(), 0).unwrap();
        assert_eq!(
            request.status().unwrap(),
            if redirects == 0 { 302 } else { 200 }
        );
        serving.join().unwrap();
    }
    let (server, url) = listener();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let serving = thread::spawn(move || {
        let (_stream, _) = server.accept().unwrap();
        // Hold TCP open so an unrelated peer close cannot mask the short body.
        let _ = held.recv_timeout(Duration::from_secs(3));
    });
    let mut client = Client::new().unwrap();
    let mut request = client
        .request(
            "POST",
            &url,
            "",
            Duration::from_secs(1),
            Duration::from_secs(3),
            0,
        )
        .unwrap();
    assert_eq!(
        request.send(&b"short"[..], 10).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    drop(request);
    let _ = release.send(());
    serving.join().unwrap();
}

#[test]
fn integrated_proxy_refusal_is_bounded_and_never_falls_back_to_basic() {
    let (server, _) = listener();
    let proxy = wide(&server.local_addr().unwrap().to_string());
    server.set_nonblocking(true).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    let serving = thread::spawn(move || {
        let mut requests = 0;
        while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
            match server.accept() {
                Ok((mut stream, _)) => {
                    let header = headers(&mut stream).to_lowercase();
                    assert!(header.starts_with("connect "));
                    assert!(!header.contains("proxy-authorization: basic "));
                    requests += 1;
                    assert!(requests <= 10, "Unbounded proxy authentication retries");
                    stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Negotiate\r\nProxy-Authenticate: NTLM\r\nProxy-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        }
        requests
    });
    let session = Handle::new(unsafe {
        WinHttpOpen(
            wide("Whispertype proxy test").as_ptr(),
            WINHTTP_ACCESS_TYPE_NAMED_PROXY,
            proxy.as_ptr(),
            null(),
            WINHTTP_FLAG_ASYNC,
        )
    })
    .unwrap();
    option(
        session.0,
        WINHTTP_OPTION_DISABLE_PROXY_AUTH_SCHEMES,
        WINHTTP_PROXY_DISABLE_SCHEME_BASIC | WINHTTP_PROXY_DISABLE_SCHEME_DIGEST,
    )
    .unwrap();
    let mut client = Client {
        connection: None,
        session,
    };
    let mut request = client
        .request(
            "GET",
            "https://api.groq.com/openai/v1/models",
            "",
            Duration::from_secs(1),
            Duration::from_millis(750),
            0,
        )
        .unwrap();
    let result = request.send(std::io::empty(), 0);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let requests = serving.join().unwrap();
    assert!((1..=10).contains(&requests));
    match result {
        Ok(()) => assert_eq!(request.status().unwrap(), 407),
        Err(e) => assert!(e.to_string().contains(PROXY_REFUSED), "{e}"),
    }
}

#[test]
#[ignore = "Uses live HTTPS endpoints without a real API key; no audio upload"]
fn winhttp_live_https_smoke() {
    let mut client = Client::new().unwrap();
    for (url, expected) in [
        ("https://api.groq.com/openai/v1/models", 401),
        (
            "https://api.github.com/repos/dayblox/whispertype/releases/latest",
            200,
        ),
    ] {
        let mut request = client
            .request(
                "GET",
                url,
                "",
                Duration::from_secs(10),
                Duration::from_secs(20),
                0,
            )
            .unwrap();
        request.send(std::io::empty(), 0).unwrap();
        assert_eq!(request.status().unwrap(), expected);
        assert!(
            std::io::copy(&mut request.take(128_001), &mut std::io::sink()).unwrap() <= 128_000
        );
    }
}

#[test]
#[ignore = "Offline upload allocation check; run alone in release mode"]
fn winhttp_upload_allocations_stay_bounded() {
    for length in [64 * 1024, 8 * 1024 * 1024] {
        let (server, url) = listener();
        let serving = thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            headers(&mut stream);
            let mut remaining = length;
            let mut buffer = [0; 8192];
            while remaining > 0 {
                let limit = buffer.len().min(remaining);
                let n = stream.read(&mut buffer[..limit]).unwrap();
                assert!(n > 0);
                remaining -= n;
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let mut client = Client::new().unwrap();
        let mut request = client
            .request(
                "POST",
                &url,
                "",
                Duration::from_secs(1),
                Duration::from_secs(5),
                0,
            )
            .unwrap();
        let (allocations, peak) = crate::audio::test_allocations(|| {
            request.send(std::io::repeat(42), length as u64).unwrap();
        });
        assert!(peak <= 4096, "Upload buffered audio: {peak}");
        serving.join().unwrap();
        println!("WinHTTP upload bytes: {length}, Rust heap allocations: {allocations}, tracked peak: {peak}; fixed 64 KiB stack buffer. Includes mock-server headers, excludes Windows HTTP/TLS.");
    }
}
