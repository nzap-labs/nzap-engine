# Architecture

NZAP Engine is one desktop process: a React UI in the system webview and a
Rust engine behind it. There is no server, database or account system.
Google Colab provides the compute; GitHub hosts the public notebooks.

```
 WebView (React 19, TanStack Router + Query)
    │  invoke(command, args)          ▲  Channel<event>  (streams)
    ▼                                 │
 src-tauri  — thin adapter: commands/*, state.rs, tray.rs, plugins
    │
    ▼
 crates/nzap-core  — the engine, no Tauri dependency
    auth/       PKCE, loopback + copy/paste flows, refresh, revoke
    secrets.rs  OS keychain, 0600 file fallback
    colab/      front door (/tun/m/*), v1 APIs, quota, resources
    runtime/    Jupyter contents + kernels over the runtime proxy, kernel WS, TTY
    session/    runtime lifecycle, persistence, keep-alive, Drive consent
    history.rs  per-runtime JSONL log, export to ipynb/md/txt/jsonl
    ops/        automation, run a local file, ephemeral jobs, import from URL
    notebooks/  GitHub catalog (verified by SHA-256) + local notebook store
    settings.rs, paths.rs, config.rs (every Google endpoint in one place)
    │
    ▼  HTTPS / WSS
 accounts.google.com · colab.research.google.com · colab.pa.googleapis.com
 runtime proxies · Drive v3 · raw.githubusercontent.com
```

## Why these boundaries

**IPC instead of a localhost HTTP server.** An open local port can be reached by
every process and every web page on the machine (CSRF, DNS rebinding). Tauri IPC
is reachable only from the app's own webview. The only socket the app opens is
the OAuth loopback listener, which accepts one request and closes.

**`nzap-core` has no Tauri dependency.** The engine is tested with plain
`cargo test` against `nzap-mock-colab`, a mock of every Google endpoint the
engine uses. The desktop crate stays a thin adapter.

**The webview holds no secrets.** Tokens never cross IPC. The UI sees an
identity (email, name, picture) and a status; the engine attaches tokens to
outgoing requests.

## IPC conventions

- Commands are `snake_case` (`session_create`, `files_list`). Arguments and
  results are camelCase JSON.
- Errors reject with `{ code, message, status? }`. `code` is a stable string
  (`not_connected`, `auth_expired`, `too_many_runtimes`, `quota`, `not_found`,
  `invalid_input`, `colab`, `runtime`, `network`, `cancelled`, …) and the UI
  maps it to copy. See `crates/nzap-core/src/error.rs`.
- Long-running work (cell execution, notebooks, jobs, terminal output) streams
  through a `tauri::ipc::Channel`. Each stream has a `streamId`, and
  `stream_cancel(streamId)` stops it.
- Uploads send raw bytes (`files_upload_bytes`) with the session and path in
  headers, so large files are not base64-encoded into JSON.

`src/lib/ipc.ts` wraps all of this (`call`, `stream`, `EngineError`), and
`src/api/*` exposes it as TanStack Query hooks.

## A runtime's life

1. **Create**: the engine asks Colab's front door for an assignment (GET for an
   XSRF token, then POST). HTTP 412 means too many runtimes, and a 400 naming an
   accelerator means quota.
2. **Connect**: it opens a Jupyter session and kernel through the runtime proxy
   and a WebSocket to the kernel. The proxy token goes in a header and a query
   parameter, as Colab expects.
3. **Use**: cells run over the kernel channel. `input()` and Colab requests
   such as `drive.mount()` pause the cell and ask the UI; replies go back on
   the same channel. Every execution is appended to the runtime's history.
4. **Keep alive**: while the app runs, it pings each runtime at the configured
   interval (30–600 s), for at most 24 hours per runtime.
5. **Stop**: the engine releases the assignment. Closing the app does *not*
   release runtimes. On the next launch the app lists them again and reconnects.

## The simulated engine

`src/dev/fake-engine.ts` implements every IPC command in memory with the same
error codes. It powers `npm run dev` in a plain browser, the Vitest component
tests and the Playwright suite, so UI work needs neither Rust nor Google.
When you add a command, add it there too.

## Repository layout

```
src/                 React app (routes/, features/, api/, components/, lib/, dev/)
src-tauri/           Tauri crate: commands/, state.rs, tray.rs, tauri.conf.json
crates/nzap-core/    engine + integration tests (tests/)
crates/nzap-mock-colab/  axum mock of Google OAuth, Colab, Jupyter, TTY, Drive
e2e/web/             Playwright against the simulated engine
e2e/desktop/         WebdriverIO + tauri-driver against the real app and the mock
docs/                user and maintainer documentation
```
