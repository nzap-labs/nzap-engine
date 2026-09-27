# NZAP Engine

**Your own Google Colab runtimes, from a native window.**

NZAP Engine is the open-source desktop edition of NZAP. It allocates and drives
the Colab VMs that belong to _your_ Google account (CPU, GPU and TPU) and gives
you a console, a real terminal, a file manager, parameterised notebooks and
ephemeral jobs. There is no server, no NZAP account and no database: install one
package and connect Google.

> **Status: early development.** The build is proceeding in phases. See
> [PLAN.md](./PLAN.md) for the architecture, the feature-parity map and
> progress. Nothing is released yet.

## What it does

- **Connect Google once.** OAuth with PKCE in your browser. The refresh token
  stays in your OS keychain and never reaches the UI.
- **Runtimes.** Assign CPU / T4 / L4 / G4 / A100 / H100 / TPU v5e / v6e, with
  optional High-RAM, then keep them alive, restart, interrupt and release them.
  Runtimes created elsewhere can be imported.
- **Console.** Streaming cell output, `input()` prompts, and Drive / Google Cloud
  consent that pauses the cell and resumes it after you approve.
- **Setup.** Install packages (uv → pip), mount Drive, authenticate Google Cloud.
- **Terminal.** A real shell on the VM.
- **Files.** Browse, edit, upload, download, rename, create and delete.
- **Run.** Run a local `.py`/`.ipynb` (or a Colab / Drive / GitHub link) and get
  the executed notebook back, or launch an ephemeral job that allocates a fresh VM,
  runs a script, returns its artifacts and releases the VM.
- **Notebooks.** A public collection hosted on GitHub
  ([nzap-labs/nzap-notebooks](https://github.com/nzap-labs/nzap-notebooks)) plus
  your own private notebooks, all with typed parameters.
- **Compute units.** Burn rate, free time left and low-balance alerts.
- **History.** Every cell and operation, exportable as `.ipynb`, `.md`, `.txt`
  or `.jsonl`.

## How it works

```
NZAP Engine (one process)
├─ WebView: React UI  ──IPC──►  Tauri shell  ──►  nzap-core (Rust)
└──────────────────────────────────────────────────────┬───────────
                                        HTTPS / WSS to Google Colab,
                                        your runtimes, Drive, GitHub
```

The engine is a Rust port of `colab-studio`, which speaks the same wire protocol
as Google's own `google-colab-cli` and the Colab VS Code extension.

## Development

Prerequisites: Node 22+, the Rust stable toolchain, and the
[Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run app:dev      # desktop app with hot reload
npm run dev          # frontend only, in a browser
```

| Command                                              | What it does                      |
| ---------------------------------------------------- | --------------------------------- |
| `npm run lint` / `typecheck` / `test`                | frontend checks and unit tests    |
| `npm run build`                                      | production frontend bundle        |
| `npm run app:build`                                  | installers for the current OS     |
| `cargo test --workspace`                             | engine unit and integration tests |
| `cargo clippy --workspace --all-targets -D warnings` | Rust lints                        |

Layout: `src/` (React UI), `src-tauri/` (desktop shell), `crates/nzap-core`
(the engine), `crates/nzap-mock-colab` (mock Google services for tests), `e2e/`.

## Disclaimer

NZAP Engine is not affiliated with or endorsed by Google. It uses the same
endpoints as Google's Colab clients, and those are internal APIs that may
change without notice. Your use of Colab is governed by Google's terms.

## License

[Apache-2.0](./LICENSE)
