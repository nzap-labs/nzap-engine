//! Import a notebook or script from a link — port of colab-vscode's
//! `colab.importNotebookFromUrl` (`src/drive/commands/import.ts`, via
//! colab-studio `runfile.py`).
//!
//! * Colab `/drive/<id>` and Drive `/file/d/<id>` / `/open?id=` links are
//!   read through Drive v3 (`files/<id>?alt=media`) with the user's token;
//! * Colab `/github/…` and GitHub `blob` links resolve to
//!   raw.githubusercontent.com;
//! * any other public `https://` URL is fetched anonymously.
//!
//! Non-Drive fetches refuse private, loopback and link-local targets (the
//! import must not become a way to reach services on the user's network)
//! and every download is capped at 20 MB.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::Serialize;

use crate::auth::AuthManager;
use crate::error::{Error, Result};

pub const MAX_IMPORT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Drive { id: String },
    Http { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFile {
    pub filename: String,
    /// `ipynb` | `py`
    pub kind: String,
    pub content: String,
    pub size: usize,
}

fn is_id(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn github_raw(owner: &str, repo: &str, rest: &str) -> Source {
    Source::Http { url: format!("https://raw.githubusercontent.com/{owner}/{repo}/{rest}") }
}

/// `/<owner>/<repo>/blob/<rest>` → `(owner, repo, rest)`.
fn blob_parts(path: &str) -> Option<(&str, &str, &str)> {
    let mut parts = path.trim_start_matches('/').splitn(4, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    (parts.next()? == "blob").then_some(())?;
    let rest = parts.next()?;
    (!owner.is_empty() && !repo.is_empty() && !rest.is_empty()).then_some((owner, repo, rest))
}

/// Classify a link (import.ts `resolveRemoteSource`, plus GitHub links).
pub fn resolve_source(link: &str) -> Result<Source> {
    let text = link.trim();
    if text.is_empty() {
        return Err(Error::invalid("Paste a notebook link."));
    }
    let text = if text.starts_with("http://") || text.starts_with("https://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let url = url::Url::parse(&text).map_err(|_| Error::invalid("That is not a valid link."))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path();

    match host.as_str() {
        "colab.research.google.com" => {
            if let Some(id) = path.strip_prefix("/drive/") {
                let id = id.split('/').next().unwrap_or_default();
                if is_id(id) {
                    return Ok(Source::Drive { id: id.to_owned() });
                }
            }
            if let Some(rest) = path.strip_prefix("/github") {
                if let Some((owner, repo, rest)) = blob_parts(rest) {
                    return Ok(github_raw(owner, repo, rest));
                }
            }
            Err(Error::invalid("Unsupported Colab link (use a /drive/… or /github/… URL)."))
        }
        "drive.google.com" => {
            if let Some(id) = path.strip_prefix("/file/d/") {
                let id = id.split('/').next().unwrap_or_default();
                if is_id(id) {
                    return Ok(Source::Drive { id: id.to_owned() });
                }
            }
            if path == "/open" {
                if let Some((_, id)) = url.query_pairs().find(|(key, _)| key == "id") {
                    if is_id(&id) {
                        return Ok(Source::Drive { id: id.into_owned() });
                    }
                }
            }
            Err(Error::invalid("Unsupported Drive link."))
        }
        "github.com" if blob_parts(path).is_some() => {
            let (owner, repo, rest) = blob_parts(path).unwrap_or_default();
            Ok(github_raw(owner, repo, rest))
        }
        _ if url.scheme() != "https" => Err(Error::invalid("Only https:// links can be imported.")),
        _ => Ok(Source::Http { url: url.into() }),
    }
}

/// Whether an address is publicly routable (the SSRF guard).
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        || (a == 198 && (b == 18 || b == 19)))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || first == 0x2001 && ip.segments()[1] == 0x0db8)
}

async fn assert_public_host(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|_| Error::invalid("That is not a valid link."))?;
    let host = parsed.host_str().ok_or_else(|| Error::invalid("That link has no host."))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addresses: Vec<_> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port))
        .await
        .map_err(|_| Error::invalid(format!("Cannot resolve {host}.")))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public(address.ip())) {
        return Err(Error::invalid("That link points at a private network address."));
    }
    Ok(())
}

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_IMPORT_BYTES {
            return Err(Error::invalid("That file is larger than 20 MB."));
        }
    }
    Ok(body)
}

fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.path_segments().and_then(|mut segments| segments.next_back().map(str::to_owned)))
        .filter(|name| !name.is_empty())
        .map(|name| percent_encoding::percent_decode_str(&name).decode_utf8_lossy().into_owned())
        .unwrap_or_else(|| "imported.ipynb".to_owned())
}

/// Fetch policy knobs; tests allow loopback so they can use the mock.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    pub allow_private_hosts: bool,
}

/// Fetch a notebook or script. The Google token is only ever sent to the
/// Drive API.
pub async fn import_from_url(auth: &AuthManager, link: &str, options: ImportOptions) -> Result<ImportedFile> {
    let http = auth.http();
    let (filename, raw) = match resolve_source(link)? {
        Source::Drive { id } => {
            let token = auth.access_token().await.map_err(|error| match error {
                Error::NotConnected => Error::invalid("Connect Google to import from Drive."),
                other => other,
            })?;
            let files = auth.endpoints().drive_files.trim_end_matches('/').to_owned();
            let meta = http
                .get(format!("{files}/{id}"))
                .bearer_auth(&token)
                .query(&[("fields", "name")])
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await?;
            if matches!(meta.status().as_u16(), 403 | 404) {
                return Err(Error::invalid(
                    "Google Drive refused that file. This app's Drive access is limited to \
                     files it created or you opened with it (drive.file scope); share the \
                     notebook publicly or download and upload it instead.",
                ));
            }
            if !meta.status().is_success() {
                return Err(Error::Network(format!("Drive answered {}.", meta.status().as_u16())));
            }
            let meta: serde_json::Value = meta.json().await?;
            let filename = meta
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{id}.ipynb"));
            let media = http
                .get(format!("{files}/{id}"))
                .bearer_auth(&token)
                .query(&[("alt", "media")])
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await?;
            if !media.status().is_success() {
                return Err(Error::Network(format!("Drive answered {}.", media.status().as_u16())));
            }
            (filename, read_limited(media).await?)
        }
        Source::Http { url } => {
            // Redirects are followed by hand (once) so each hop is checked.
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(format!("nzap-engine/{}", crate::VERSION))
                .build()?;
            if !options.allow_private_hosts {
                assert_public_host(&url).await?;
            }
            let timeout = std::time::Duration::from_secs(60);
            let mut response = client.get(&url).timeout(timeout).send().await?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|location| url::Url::parse(&url).ok()?.join(location).ok())
                    .ok_or_else(|| Error::invalid("That link redirects nowhere."))?;
                if location.scheme() != "https" && !options.allow_private_hosts {
                    return Err(Error::invalid("That link redirects to a non-https address."));
                }
                if !options.allow_private_hosts {
                    assert_public_host(location.as_str()).await?;
                }
                response = client.get(location.as_str()).timeout(timeout).send().await?;
            }
            if !response.status().is_success() {
                return Err(Error::invalid(format!(
                    "Could not fetch that link ({}).",
                    response.status().as_u16()
                )));
            }
            (filename_from_url(&url), read_limited(response).await?)
        }
    };

    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut filename = filename;
    let kind = if filename.to_ascii_lowercase().ends_with(".ipynb") {
        super::runfile::load_notebook(&serde_json::Value::String(text.clone()))?;
        "ipynb"
    } else if text.trim_start().starts_with('{') && text.contains("\"cells\"") {
        filename.push_str(".ipynb");
        "ipynb"
    } else {
        "py"
    };
    Ok(ImportedFile { filename, kind: kind.to_owned(), content: text, size: raw.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(id: &str) -> Source {
        Source::Drive { id: id.into() }
    }

    fn http(url: &str) -> Source {
        Source::Http { url: url.into() }
    }

    #[test]
    fn resolves_every_link_shape() {
        let cases = [
            ("https://colab.research.google.com/drive/1AbC_d-9", drive("1AbC_d-9")),
            ("colab.research.google.com/drive/1AbC#scrollTo=x", drive("1AbC")),
            ("https://drive.google.com/file/d/XyZ/view?usp=sharing", drive("XyZ")),
            ("https://drive.google.com/open?id=QwE", drive("QwE")),
            (
                "https://colab.research.google.com/github/nzap-labs/nzap-notebooks/blob/main/a/b.ipynb",
                http("https://raw.githubusercontent.com/nzap-labs/nzap-notebooks/main/a/b.ipynb"),
            ),
            (
                "https://github.com/o/r/blob/main/x.py",
                http("https://raw.githubusercontent.com/o/r/main/x.py"),
            ),
            ("https://example.com/n.ipynb", http("https://example.com/n.ipynb")),
        ];
        for (link, expected) in cases {
            assert_eq!(resolve_source(link).unwrap(), expected, "{link}");
        }
        for bad in [
            "",
            "http://example.com/n.ipynb",
            "https://colab.research.google.com/notebooks/intro.ipynb",
            "https://drive.google.com/drive/folders/abc",
            "https://drive.google.com/open?id=../x",
        ] {
            assert!(resolve_source(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ssrf_guard() {
        for private in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0",
            "100.64.0.1", "224.0.0.1", "255.255.255.255", "::1", "fe80::1", "fd00::1",
            "::ffff:127.0.0.1", "2001:db8::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["8.8.8.8", "140.82.112.3", "2606:4700::6810:84e5"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn filenames() {
        assert_eq!(filename_from_url("https://x.io/a/my%20nb.ipynb"), "my nb.ipynb");
        assert_eq!(filename_from_url("https://x.io/"), "imported.ipynb");
    }
}
