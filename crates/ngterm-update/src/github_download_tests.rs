use super::*;
use std::io::BufRead as _;

struct Server {
    url: String,
    requests: std::thread::JoinHandle<Vec<String>>,
}

impl Server {
    fn new(responses: Vec<&str>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/latest", listener.local_addr().unwrap());
        let responses: Vec<_> = responses.into_iter().map(str::to_owned).collect();
        let requests = std::thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| serve(&listener, &response))
                .collect()
        });
        Self { url, requests }
    }

    fn finish(self) -> Vec<String> {
        self.requests.join().unwrap()
    }
}

fn accept(listener: &std::net::TcpListener) -> std::net::TcpStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(std::time::Instant::now() < deadline, "missing request");
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => panic!("accept: {e}"),
        }
    }
}

fn serve(listener: &std::net::TcpListener, response: &str) -> String {
    let mut stream = accept(listener);
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = std::io::BufReader::new(&mut stream);
    let mut request = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
        request.push_str(&line);
    }
    write!(
        stream,
        "HTTP/1.1 {response}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )
    .unwrap();
    request.to_ascii_lowercase()
}

fn test_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

const LIMITED: &str = "403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 0";

#[test]
fn a_success_or_unrelated_refusal_never_reads_credentials() {
    for (status, success) in [
        ("200 OK", true),
        ("403 Forbidden", false),
        ("429 Too Many Requests", false),
        ("403 Forbidden\r\nX-RateLimit-Remaining: 3", false),
        ("403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nRetry-After: 60", false),
    ] {
        let server = Server::new(vec![status]);
        let result = metadata_response(&test_agent(), &server.url, || panic!("must not read credentials"));
        assert_eq!(result.is_ok(), success);
        assert!(!server.finish()[0].contains("authorization:"));
    }
}

#[test]
fn only_the_single_retry_carries_authorization() {
    for status in [LIMITED, "429 Too Many Requests\r\nX-RateLimit-Remaining: 0"] {
        let server = Server::new(vec![status, "200 OK"]);
        assert!(metadata_response(&test_agent(), &server.url, || Some("test-token".into())).is_ok());
        let requests = server.finish();
        assert!(!requests[0].contains("authorization:"));
        assert!(requests[1].contains("authorization: bearer test-token\r\n"));
    }
}

#[test]
fn no_token_and_failed_authentication_stop_with_a_useful_error() {
    let server = Server::new(vec![LIMITED]);
    let error = metadata_response(&test_agent(), &server.url, || None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("rate limit"));
    assert!(error.contains("Retry in 0 seconds"));
    server.finish();
    for second in ["401 Unauthorized", LIMITED] {
        let server = Server::new(vec![LIMITED, second]);
        let error = metadata_response(&test_agent(), &server.url, || Some("secret-token".into()))
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-token"));
        assert_eq!(server.finish().len(), 2);
    }
}

#[test]
fn metadata_redirects_are_not_followed_even_after_authentication() {
    let redirect = "302 Found\r\nLocation: http://127.0.0.1:1/must-not-contact";
    let server = Server::new(vec![redirect]);
    assert!(matches!(
        metadata_response(&test_agent(), &server.url, || panic!("no token")),
        Err(Error::Http(ureq::Error::StatusCode(302)))
    ));
    server.finish();
    let server = Server::new(vec![LIMITED, redirect]);
    assert!(matches!(
        metadata_response(&test_agent(), &server.url, || Some("test-token".into())),
        Err(Error::Http(ureq::Error::StatusCode(302)))
    ));
    assert_eq!(server.finish().len(), 2);
}
