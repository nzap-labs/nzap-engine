//! The OAuth loopback redirect target (colab-vscode's `LocalServerFlow`).
//!
//! A one-shot HTTP listener bound to the loopback interface on an ephemeral
//! port. Google's installed-app clients accept any port for loopback
//! redirects. The listener answers exactly one valid callback (the one
//! carrying our `state`), ignores everything else, and is dropped
//! afterwards; it never outlives a sign-in attempt.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{Error, Result};

/// How long a sign-in may take before the listener gives up.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const CALLBACK_PATH: &str = "/callback";

pub struct LoopbackServer {
    v4: TcpListener,
    v6: Option<TcpListener>,
    port: u16,
}

impl LoopbackServer {
    /// Bind `127.0.0.1` on a free port, and `::1` on the same port when the
    /// host has IPv6 loopback (browsers may resolve `localhost` to either).
    pub async fn bind() -> Result<Self> {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = v4.local_addr()?.port();
        let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok();
        Ok(Self { v4, v6, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://localhost:<port>/callback` — the loopback form colab-studio
    /// verified against the default client.
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}{CALLBACK_PATH}", self.port)
    }

    /// Wait for Google's redirect and return the authorization code.
    pub async fn wait_for_code(self, expected_state: &str, timeout: Duration) -> Result<String> {
        tokio::time::timeout(timeout, self.accept_loop(expected_state)).await.map_err(|_| {
            Error::Auth("Timed out waiting for Google sign-in. Please try again.".to_owned())
        })?
    }

    async fn accept_loop(&self, expected_state: &str) -> Result<String> {
        loop {
            let stream = match &self.v6 {
                Some(v6) => tokio::select! {
                    accepted = self.v4.accept() => accepted?.0,
                    accepted = v6.accept() => accepted?.0,
                },
                None => self.v4.accept().await?.0,
            };
            match handle_connection(stream, expected_state).await {
                Ok(Some(outcome)) => return outcome,
                Ok(None) => continue,
                Err(error) => {
                    tracing::debug!("Ignoring a broken loopback request: {error}");
                    continue;
                }
            }
        }
    }
}

/// Serve one request. `Ok(None)` means "not our callback, keep waiting".
async fn handle_connection(
    mut stream: TcpStream,
    expected_state: &str,
) -> std::io::Result<Option<Result<String>>> {
    let head = match tokio::time::timeout(Duration::from_secs(10), read_head(&mut stream)).await {
        Ok(head) => head?,
        Err(_) => return Ok(None),
    };
    let Some(target) = request_target(&head) else {
        respond(&mut stream, 400, "Bad request", "This address only accepts Google sign-in.")
            .await?;
        return Ok(None);
    };
    let Ok(url) = url::Url::parse(&format!("http://localhost{target}")) else {
        respond(&mut stream, 400, "Bad request", "Malformed request.").await?;
        return Ok(None);
    };
    if url.path() != CALLBACK_PATH {
        respond(&mut stream, 404, "Not found", "Nothing here.").await?;
        return Ok(None);
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    // A request without our state is not Google's redirect for this attempt
    // (a stale tab, or another page probing the port): answer and keep waiting.
    if !state.as_deref().is_some_and(|state| constant_time_eq(state, expected_state)) {
        respond(
            &mut stream,
            400,
            "Sign-in link expired",
            "This sign-in link is no longer valid. Start connecting again from NZAP Engine.",
        )
        .await?;
        return Ok(None);
    }

    if let Some(error) = error {
        respond(
            &mut stream,
            400,
            "Sign-in cancelled",
            "Google sign-in was cancelled. You can close this tab.",
        )
        .await?;
        return Ok(Some(Err(Error::Auth(format!("Google sign-in was cancelled ({error}).")))));
    }

    let Some(code) = code.filter(|code| !code.is_empty()) else {
        respond(&mut stream, 400, "Sign-in failed", "Google did not return a code.").await?;
        return Ok(Some(Err(Error::Auth(
            "Google did not return an authorization code.".to_owned(),
        ))));
    };

    respond(
        &mut stream,
        200,
        "Google connected",
        "You can close this tab and return to NZAP Engine.",
    )
    .await?;
    Ok(Some(Ok(code)))
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n")
            || buffer.len() >= MAX_REQUEST_BYTES
        {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// The target of a `GET <target> HTTP/1.x` request line.
fn request_target(head: &str) -> Option<&str> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET" && target.starts_with('/')).then_some(target)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    title: &str,
    detail: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = page(status == 200, title, detail);
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// The page the browser shows after the redirect, in NZAP's paper palette.
fn page(ok: bool, title: &str, detail: &str) -> String {
    let dot = if ok { "#6ece9d" } else { "#e78b72" };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#f8f5ed;color:#11110f;\
font-family:'DM Sans',ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,sans-serif}}\
.card{{border:1px solid #11110f;border-radius:24px;padding:40px 48px;max-width:440px;text-align:center}}\
.dot{{display:inline-block;width:12px;height:12px;border-radius:50%;background:{dot}}}\
h1{{font-size:22px;font-weight:500;margin:16px 0 8px}}p{{color:#6f706b;margin:0;line-height:1.5}}\
@media (prefers-color-scheme:dark){{body{{background:#0f0f0e;color:#f0eee6}}.card{{border-color:#f0eee6}}p{{color:#9a9b93}}}}\
</style></head><body><div class=\"card\"><span class=\"dot\"></span><h1>{title}</h1><p>{detail}</p></div></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn returns_the_code_for_our_state_only() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();
        assert!(server.redirect_uri().starts_with("http://localhost:"));
        let waiter = tokio::spawn(async move { server.wait_for_code("good", LOGIN_TIMEOUT).await });

        assert!(get(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(get(port, "/callback?code=x&state=bad").await.starts_with("HTTP/1.1 400"));
        let ok = get(port, "/callback?code=the-code&state=good").await;
        assert!(ok.starts_with("HTTP/1.1 200"));
        assert!(ok.contains("Google connected"));

        assert_eq!(waiter.await.unwrap().unwrap(), "the-code");
    }

    #[tokio::test]
    async fn reports_cancellation() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();
        let waiter = tokio::spawn(async move { server.wait_for_code("s", LOGIN_TIMEOUT).await });
        get(port, "/callback?error=access_denied&state=s").await;
        let error = waiter.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn times_out() {
        let server = LoopbackServer::bind().await.unwrap();
        let error = server.wait_for_code("s", Duration::from_millis(50)).await.unwrap_err();
        assert!(matches!(error, Error::Auth(_)));
    }

    #[test]
    fn parses_request_lines() {
        assert_eq!(request_target("GET /callback?a=1 HTTP/1.1\r\n"), Some("/callback?a=1"));
        assert_eq!(request_target("POST /callback HTTP/1.1\r\n"), None);
        assert_eq!(request_target(""), None);
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}
