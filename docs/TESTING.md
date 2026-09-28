# Testing

Every layer has automated tests, and none of them needs a Google account.
The live checklist at the end covers what only real Colab can prove.

| Suite                                    | What it covers                                                                                                        | Command                                                 |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| Engine (Rust)                            | OAuth, Colab wire contract, runtimes, kernel, terminal, files, notebooks, jobs — against `nzap-mock-colab`            | `cargo test --workspace`                                |
| Components (Vitest)                      | UI components against the simulated engine                                                                            | `npm test`                                              |
| Web E2E (Playwright)                     | Every workspace flow in Chromium and WebKit, simulated engine                                                         | `npx playwright test`                                   |
| Desktop E2E (WebdriverIO + tauri-driver) | The real app and engine, mock Google: sign-in, runtime, cells, `input()`, notebook, terminal, files, stop, disconnect | see [`e2e/desktop/README.md`](../e2e/desktop/README.md) |

CI runs all of them on every push and pull request. The engine tests run on
Linux, Windows and macOS, and the desktop E2E suite runs on Linux and Windows
(macOS has no WebDriver for WKWebView).

## The two fakes

- **`crates/nzap-mock-colab`** is an axum server that speaks Google's OAuth,
  Colab's front door and v1 APIs, the Jupyter REST API and kernel WebSocket,
  `/colab/tty` and Drive. It has a scripted "kernel" that understands a small
  set of statements (prints, `input()`, `drive.mount()`, `sys.exit`, errors).
  Tests turn knobs on its state to produce quota errors, consent prompts,
  slow keep-alives and so on. `cargo run -p nzap-mock-colab --bin mock-colab`
  starts it on port 9901.
- **`src/dev/fake-engine.ts`** answers every IPC command in the browser. It
  backs `npm run dev`, Vitest and Playwright. Tests drive it through
  `window.__NZAP_FAKE__`.

## Pointing a debug build at the mock

Debug builds read these variables (release builds ignore them):

| Variable            | Effect                                                    |
| ------------------- | --------------------------------------------------------- |
| `NZAP_MOCK_GOOGLE`  | Send every Google request to this base URL                |
| `NZAP_DATA_DIR`     | Keep all app data under this directory                    |
| `NZAP_NO_KEYCHAIN`  | Store the refresh token in a file instead of the keychain |
| `NZAP_E2E_OPEN_LOG` | Append URLs to this file instead of opening a browser     |

## Live verification checklist

Run this against real Google before a release, with a fresh data directory,
on at least one OS per release and on all three for major changes.

**Sign-in**

- [ ] Connect Google in the browser flow; the Account page shows your name, email and picture.
- [ ] Disconnect, then connect with **Use a code**.
- [ ] Quit and relaunch: still connected, with no browser prompt.
- [ ] Revoke the app at myaccount.google.com/permissions; the app asks you to reconnect.

**Runtimes**

- [ ] The compute card shows the Colab plan and compute units.
- [ ] Launch a CPU runtime and run `print(1)`.
- [ ] Launch a T4 GPU runtime and run `!nvidia-smi` (Colab Pro, or when free GPUs are available).
- [ ] A long cell streams output live, and **Interrupt** stops it.
- [ ] `input()` prompts in the console and receives the answer.
- [ ] `from google.colab import drive; drive.mount('/content/drive')` asks for consent, then mounts.
- [ ] Restart the kernel; variables are gone.
- [ ] Quit with a runtime running, relaunch: the runtime reconnects.
- [ ] Stop and release: the runtime disappears from colab.research.google.com too.

**Workspace**

- [ ] Terminal: `whoami`, `ls /content`, a full-screen program (`top`), resize the window.
- [ ] Files: upload, download, rename, delete, new folder, edit a text file.
- [ ] Resources show RAM, disk and (on GPU) GPU memory.
- [ ] History exports as `.ipynb` and opens in Jupyter.
- [ ] Run a local `.py` file, and import a notebook from a GitHub URL.
- [ ] Run an ephemeral job that writes an artifact; the artifact lands in the artifacts folder.

**Notebooks**

- [ ] Run a public notebook with parameters.
- [ ] Fork it, edit it, run your copy, export it, delete it, import the export.

**App**

- [ ] Close-to-tray: closing hides the window, the tray brings it back, and **Quit** exits.
- [ ] Light and dark themes; keyboard-only navigation through the sidebar and tabs.
- [ ] Settings → Open log folder opens the logs.
