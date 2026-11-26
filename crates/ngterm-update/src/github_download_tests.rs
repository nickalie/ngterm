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
    assert_eq!(
        error,
        "GitHub's API rate limit was reached. Retry in 0 seconds. Check local GitHub authentication or use Release \
         Downloads."
    );
    server.finish();
    let refused = "GitHub's API rate limit was reached. Retry in 0 seconds. GitHub refused the local token \
                   (GH_TOKEN, GITHUB_TOKEN or gh auth token). Check local GitHub authentication or use Release \
                   Downloads.";
    for (second, expected) in [
        ("401 Unauthorized", refused),
        ("403 Forbidden", refused),
        (
            LIMITED,
            "GitHub's API rate limit was reached. Retry in 0 seconds. Check local GitHub authentication or use \
             Release Downloads.",
        ),
    ] {
        let server = Server::new(vec![LIMITED, second]);
        let error = metadata_response(&test_agent(), &server.url, || Some("secret-token".into()))
            .unwrap_err()
            .to_string();
        assert_eq!(error, expected);
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

#[test]
fn a_credential_that_is_not_a_header_value_is_never_sent() {
    let server = Server::new(vec![LIMITED]);
    let error = metadata_response(&test_agent(), &server.url, || Some("bad\ntoken".into()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("rate limit"));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn only_the_github_api_reads_credentials() {
    assert_eq!(github_only(API, || Some("token".into())), Some("token".into()));
    assert_eq!(github_only(API, || None), None);
    assert_eq!(
        github_only("https://api.github.com/other", || panic!("not GitHub's API")),
        None
    );
    let server = crate::fakes::Server::start().unwrap();
    assert_eq!(github_only(&server.source().api, || panic!("not GitHub's API")), None);
}

#[test]
fn a_source_other_than_github_reports_the_limit_without_credentials() {
    let server = crate::fakes::Server::start().unwrap();
    server.route("/latest", LIMITED, "{}");
    let install = crate::tests::installation(std::path::Path::new("/tmp/ngterm"));
    let error = server
        .source()
        .check_with(&install, "0.1.0", &Trust::default(), || {
            panic!("must not read credentials")
        })
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("GitHub's API rate limit was reached. Retry in 0 seconds."));
    assert_eq!(server.requests(), ["/latest"]);
}

#[test]
fn a_limit_gives_the_wait_that_github_asks_for() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let reset = format!(
        "403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: {}",
        now + 600
    );
    for (status, wait) in [
        (reset.as_str(), 590..=600),
        ("403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nRetry-After: 60", 60..=60),
        ("429 Too Many Requests\r\nRetry-After: 120", 120..=120),
        ("403 Forbidden\r\nRetry-After: 30\r\nX-RateLimit-Reset: 1", 30..=30),
    ] {
        let server = Server::new(vec![status]);
        let error = metadata_response(&test_agent(), &server.url, || None).unwrap_err();
        server.finish();
        assert!(error.is_transient(), "{status}");
        let seconds = error.retry_after().expect("the limit gives a wait").as_secs();
        assert!(wait.contains(&seconds), "{status}: {seconds}");
        assert!(
            error.to_string().contains(&format!(" Retry in {seconds} seconds.")),
            "{error}"
        );
    }
    let server = Server::new(vec!["403 Forbidden\r\nX-RateLimit-Remaining: 0"]);
    let error = metadata_response(&test_agent(), &server.url, || None).unwrap_err();
    server.finish();
    assert_eq!(error.retry_after(), None);
    assert_eq!(
        error.to_string(),
        "GitHub's API rate limit was reached. Check local GitHub authentication or use Release Downloads."
    );
}

#[test]
fn only_a_limit_a_lost_connection_or_a_server_error_is_transient() {
    let limited = Error::RateLimited {
        message: "limited".into(),
        retry_after: None,
    };
    assert!(limited.is_transient());
    for (error, transient) in [
        (ureq::Error::StatusCode(503), true),
        (ureq::Error::StatusCode(429), true),
        (ureq::Error::ConnectionFailed, true),
        (ureq::Error::HostNotFound, true),
        (ureq::Error::Io(std::io::Error::other("reset")), true),
        (ureq::Error::StatusCode(404), false),
        (ureq::Error::StatusCode(403), false),
        (ureq::Error::TooManyRedirects, false),
    ] {
        assert_eq!(Error::Http(error).is_transient(), transient);
    }
    assert!(!crate::error("no matching asset").is_transient());
    assert!(!Error::Io(std::io::Error::other("disk full")).is_transient());
    assert_eq!(crate::error("other").retry_after(), None);
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/latest", closed.local_addr().unwrap());
    drop(closed);
    let error = metadata_response(&test_agent(), &url, || None).unwrap_err();
    assert!(error.is_transient(), "{error:?}");
}
