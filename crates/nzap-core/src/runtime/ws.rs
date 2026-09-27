//! WebSocket connections to a runtime (kernel channels, `/colab/tty`).
//!
//! TLS uses rustls with an explicit `ring` provider and the webpki root
//! store, so the engine never depends on which crypto backend other crates
//! happen to enable.

use std::sync::{Arc, OnceLock};

use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::error::{Error, Result};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("the ring provider supports TLS 1.2 and 1.3")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// Open a WebSocket to `url` with extra request headers. Error messages
/// never include the URL (it may carry a runtime-proxy token).
pub async fn connect(url: &str, headers: &[(&'static str, String)]) -> Result<WsStream> {
    let mut request = url
        .into_client_request()
        .map_err(|_| Error::internal("Invalid runtime socket URL."))?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Error::internal("Invalid socket header name."))?;
        let value =
            HeaderValue::from_str(value).map_err(|_| Error::internal("Invalid socket header value."))?;
        request.headers_mut().insert(name, value);
    }

    let mut config = WebSocketConfig::default();
    config.max_message_size = None;
    config.max_frame_size = None;

    let (stream, _response) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        Some(config),
        false,
        Some(Connector::Rustls(tls_config())),
    )
    .await
    .map_err(socket_error)?;
    Ok(stream)
}

fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> Error {
    use tokio_tungstenite::tungstenite::Error as WsError;
    match error {
        WsError::Http(response) => Error::runtime(
            Some(response.status().as_u16()),
            format!("The runtime refused the connection ({}).", response.status().as_u16()),
        ),
        WsError::Io(error) => Error::Network(format!("Could not reach the runtime: {error}")),
        WsError::Tls(error) => Error::Network(format!("TLS error: {error}")),
        other => Error::Network(format!("Runtime socket error: {other}")),
    }
}
