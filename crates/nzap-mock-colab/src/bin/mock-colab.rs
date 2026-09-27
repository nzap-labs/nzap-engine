//! A standalone mock of every Google service NZAP Engine talks to, for the
//! desktop E2E suite and for manual testing without a Google account:
//!
//! ```text
//! cargo run -p nzap-mock-colab --bin mock-colab -- 9901
//! NZAP_MOCK_GOOGLE=http://127.0.0.1:9901 npm run app:dev
//! ```
//!
//! Development builds of the app honour `NZAP_MOCK_GOOGLE`; release builds
//! ignore it.

use nzap_mock_colab::MockGoogle;

#[tokio::main]
async fn main() {
    let port = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("MOCK_COLAB_PORT").ok())
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(9901);
    let mock = MockGoogle::start_on(&format!("127.0.0.1:{port}")).await;
    println!("mock-colab listening on {}", mock.base_url);
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("mock-colab: could not wait for Ctrl-C: {error}");
        // Keep serving until killed.
        std::future::pending::<()>().await;
    }
}
