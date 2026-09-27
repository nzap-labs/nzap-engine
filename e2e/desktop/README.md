# Desktop E2E

Drives the real NZAP Engine binary through
[`tauri-driver`](https://v2.tauri.app/develop/tests/webdriver/) with WebdriverIO.
The Rust engine is real; Google is `nzap-mock-colab`. The suite covers sign-in
through the loopback redirect, a runtime, streamed cells, `input()`, a public
notebook, the terminal, files, releasing the runtime and disconnecting.

It runs on Linux and Windows. macOS has no WebDriver for WKWebView.

```bash
# from the repository root
npx tauri build --debug --no-bundle              # the app, with dev overrides enabled
cargo build -p nzap-mock-colab --bin mock-colab  # the mock Google
cargo install tauri-driver --locked

cd e2e/desktop
npm ci
# Linux: needs WebKitWebDriver (apt install webkit2gtk-driver) and a display
xvfb-run npm test
# Windows: needs an msedgedriver matching the installed WebView2
NATIVE_DRIVER=C:\path\to\msedgedriver.exe npm test
```

Debug builds read `NZAP_MOCK_GOOGLE`, `NZAP_DATA_DIR`, `NZAP_NO_KEYCHAIN` and
`NZAP_E2E_OPEN_LOG` (the app writes URLs there instead of opening a browser).
The test itself plays the browser: it fetches the consent URL, and the mock
redirects to the app's loopback listener. Release builds ignore these variables.
