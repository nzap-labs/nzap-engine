# NZAP Engine — Build Plan

NZAP Engine is the open-source, self-hosted desktop edition of NZAP: a Tauri
app for Windows, macOS and Linux that drives **your own Google Colab
runtimes** (CPU / GPU / TPU) from a native window. It keeps NZAP's UI and
everything the hosted NZAP Colab workspace offers, with none of the hosted
infrastructure. It has no NZAP account, no database and no backend server.
You install one package and connect Google.

This document is the source of truth for the build. Each phase ends with a
commit and a push to `origin/main`, and its checklist is ticked here in the same commit.

---

## 1. Principles

1. **One package, zero services.** A single installer per OS (`.msi`/`.exe`,
   `.dmg`, `.AppImage`/`.deb`/`.rpm`). No Python, no Node, no Docker and no
   local ports kept open.
2. **The user's Google account is the only identity.** No sign-up and no NZAP
   login. Connecting Google (OAuth, PKCE) is the only gate before the app is usable.
3. **No database.** Private state lives in the OS keychain and in the app's
   data directory. The public notebook collection is a GitHub repository.
4. **Same UI as NZAP.** The design tokens, sidebar shell, Colab workspace
   and panels are ported from `legacy-nzap`, with behaviour kept as-is.
5. **Same wire contract as Colab's own clients.** The engine is a faithful
   Rust port of `colab-studio`, which in turn ports `google-colab-cli` and
   `colab-vscode`. No Colab value is invented.
6. **Production grade.** Strict typing on both sides, tests at every layer,
   CI on all three OSes, signed release artefacts and a documented threat model.

## 2. From hosted NZAP to NZAP Engine

| Hosted NZAP (`legacy-nzap` + `colab-studio`)                         | NZAP Engine                                                                          |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Supabase Auth (Google sign-in) + `/login` + `/auth/callback`         | **Removed.** No app account.                                                         |
| Fastify backend: Supabase session check, token vault, proxy          | **Removed.** The Rust core runs in-process.                                          |
| `colab-studio` (Python FastAPI, upstream mode, per-user namespacing) | `crates/nzap-core` (Rust), single-user, no namespacing                               |
| Colab OAuth via backend redirect + AES-GCM vault file                | Loopback OAuth (PKCE) + refresh token in the **OS keychain**                         |
| Browser `fetch` + NDJSON streams                                     | Tauri `invoke` + typed `Channel` streams                                             |
| Terminal WebSocket + one-time tickets                                | Terminal over IPC channel (no socket exposed)                                        |
| Backend keep-alive sweep                                             | Per-runtime keep-alive tasks in the core                                             |
| `notebooks` table (public rows + private rows, RLS)                  | Public: GitHub repo `nzap-labs/nzap-notebooks` · Private: local JSON files           |
| Profile / Account / Credits (DB)                                     | Profile + Account from Google identity; the Colab plan + CCU balance replace credits |
| Landing page, `/login`, setup screen                                 | Removed (not needed in a desktop app)                                                |
| Browser downloads / uploads                                          | Native save/open dialogs; bytes stream straight to disk                              |

## 3. Architecture

```
┌──────────────────────── NZAP Engine (one process) ─────────────────────────┐
│                                                                            │
│  WebView (React 19 + TS)          Tauri shell (src-tauri)                  │
│  ─────────────────────            ───────────────────────                  │
│  NZAP design system        invoke │ commands.rs  → typed errors            │
│  Colab workspace  ────────────────► channels.rs  → Channel<Event> streams  │
│  Notebooks / Account       Channel│ state.rs     → Engine handle           │
│  TanStack Query            ◄──────┤ plugins: opener, dialog, log,          │
│                                   │ single-instance, window-state, updater │
│                                   └──────────────┬─────────────────────────┘
│                                                  │                          │
│                              crates/nzap-core (pure Rust, no Tauri deps)    │
│                              ────────────────────────────────────────────   │
│                              auth/     OAuth PKCE, loopback, refresh        │
│                              secrets/  keychain + 0600-file fallback        │
│                              colab/    /tun/m/*, v1 APIs, XSSI, XSRF dance  │
│                              quota/    CCU burn rate, free-time, severity   │
│                              runtime/  Jupyter contents/kernels/sessions    │
│                              kernel/   WS channel, multiplexed execute      │
│                              session/  lifecycle, persistence, keep-alive,  │
│                                        Drive/GCP consent pause & resume     │
│                              terminal/ /colab/tty bridge                    │
│                              history/  JSONL log + ipynb/md/txt/jsonl       │
│                              automation/ runfile/ jobs/ import/             │
│                              notebooks/ GitHub catalog + local store +      │
│                                         param validation + injection        │
└──────────────────────────────────────────────────┬──────────────────────────┘
                                                   │ HTTPS / WSS
                      accounts.google.com · colab.research.google.com
                      colab.pa.googleapis.com · runtime proxies · Drive v3
                      raw.githubusercontent.com (public notebooks)
```

**Why Tauri IPC rather than an embedded localhost HTTP server:** an open local
port can be reached by every process and every web page on the machine
(CSRF, DNS rebinding). IPC is reachable only from our own webview and is
further narrowed by Tauri capabilities. The only socket we ever open is the
short-lived OAuth loopback listener, which accepts exactly one request.

**Why `nzap-core` has no Tauri dependency:** the engine is testable with plain
`cargo test` against a mock Colab. It is also reusable later, for example by a
CLI, and the Tauri layer stays a thin adapter.

### Repository layout

```
nzap-engine/
├── src/                     React frontend (ported from legacy-nzap)
│   ├── api/                 typed IPC data access (TanStack Query)
│   ├── features/            shell, colab, notebooks, account, settings
│   ├── components/          UI primitives (Radix + Tailwind)
│   ├── lib/                 ipc, theme, format, utils
│   └── routes/              TanStack Router file routes
├── src-tauri/               Tauri app crate (commands, channels, config)
├── crates/
│   ├── nzap-core/           the engine
│   └── nzap-mock-colab/     mock Google/Colab/Jupyter/TTY servers (tests + E2E)
├── e2e/
│   ├── web/                 Playwright, UI against a mocked IPC backend
│   └── desktop/             WebdriverIO + tauri-driver, real app vs mock Colab
├── docs/                    architecture, security, OAuth, notebooks format
└── .github/                 CI, release, issue templates, dependabot
```

## 4. Feature parity map

Every capability of the hosted Colab workspace, mapped to an IPC command
(`→` marks a streaming command that reports through a `Channel`).

| Area               | Hosted route (legacy)                                                                     | Engine command                                                                                                                                                               |
| ------------------ | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Connection         | `GET /status`, `POST /connect`, callback, `POST /disconnect`                              | `auth_status`, `auth_connect`, `auth_complete_remote`, `auth_disconnect`                                                                                                     |
| Account            | `GET /account`, `GET /quota`, `GET /config`                                               | `account_get`, `quota_get`, `config_get`                                                                                                                                     |
| Runtimes           | `GET/POST /sessions`, `GET /sessions/:n`                                                  | `sessions_list`, `session_create`, `session_get`                                                                                                                             |
| Runtime ops        | connect / disconnect / keepalive / restart / interrupt / stdin / drive authorize / delete | `session_connect`, `session_disconnect`, `session_keepalive`, `session_restart`, `session_interrupt`, `session_stdin`, `session_drive_authorize`, `session_stop`             |
| Telemetry          | `GET /sessions/:n/resources`                                                              | `session_resources`                                                                                                                                                          |
| Execute            | `POST /sessions/:n/execute` (NDJSON)                                                      | → `session_execute`                                                                                                                                                          |
| Automations        | install / drivemount / gcp-auth                                                           | → `session_automation`                                                                                                                                                       |
| Run a file         | `POST /sessions/:n/run-file`                                                              | → `session_run_file`                                                                                                                                                         |
| Jobs (`colab run`) | `POST /jobs`                                                                              | → `job_run` (+ artifacts saved via dialog)                                                                                                                                   |
| Import             | `POST /import`                                                                            | `import_notebook_url`                                                                                                                                                        |
| Terminal           | ticket + WS                                                                               | → `terminal_open`, `terminal_send`, `terminal_resize`, `terminal_close`                                                                                                      |
| History            | list / export / clear                                                                     | `history_get`, `history_export` (save dialog), `history_clear`                                                                                                               |
| Files              | list / content / download / save / upload / mkdir / rename / delete                       | `files_list`, `files_read`, `files_download` (to disk), `files_write`, `files_upload` (from disk or bytes), `files_mkdir`, `files_rename`, `files_delete`                    |
| Assignments        | list / release / adopt                                                                    | `assignments_list`, `assignment_release`, `assignment_adopt`                                                                                                                 |
| Notebooks          | list / get / create / update / delete / run                                               | `notebooks_list`, `notebook_get`, `notebook_create`, `notebook_update`, `notebook_delete`, `notebook_fork`, `notebook_export`, `notebooks_refresh_catalog`, → `notebook_run` |
| Streams            | (abort fetch)                                                                             | `stream_cancel`                                                                                                                                                              |
| Settings           | —                                                                                         | `settings_get`, `settings_update` (OAuth client override, catalog URL, keep-alive, close-to-tray)                                                                            |

All IPC payloads use camelCase, and one Rust `serde` model feeds one TS type
file. Errors cross IPC as `{ code, message }`, where `code` is one of
`not_connected | auth_expired | not_found | invalid_input | too_many_runtimes |
quota | colab | runtime | io | internal`.

## 5. Data on disk

| What                            | Where                                                                      | Protection                                                                                    |
| ------------------------------- | -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Google refresh token + identity | OS keychain (Windows Credential Manager / macOS Keychain / Secret Service) | OS-level; falls back to `secrets.json` (0600) when no keychain exists, with a visible warning |
| Access token                    | memory only                                                                | refreshed 120 s before expiry                                                                 |
| Runtimes (`sessions.json`)      | `<app data>/`                                                              | 0600 because it holds runtime-proxy tokens, which are VM-scoped and short-lived               |
| History logs                    | `<app data>/history/<name>.jsonl`                                          | user data, clearable from the UI                                                              |
| Private notebooks               | `<app data>/notebooks/<id>.json`                                           | user data, exportable                                                                         |
| Public catalog cache            | `<app cache>/catalog/`                                                     | ETag-revalidated, SHA-256 verified                                                            |
| Settings                        | `<app config>/settings.json`                                               | no secrets                                                                                    |
| Logs                            | `<app log>/nzap-engine.log`                                                | rotated; tokens redacted by a log filter                                                      |

## 6. Security model (summary; full write-up in `SECURITY.md`)

- OAuth 2.0 authorization code + **PKCE S256**, `state` checked and single-use,
  loopback listener bound to `127.0.0.1`/`::1` on an ephemeral port that
  accepts one request and then closes (5-minute timeout). A copy/paste flow is kept
  as a fallback (colab-cli's remote flow).
- Tokens never reach the webview. The frontend only ever sees identity,
  status and derived data.
- Strict CSP. Tauri capabilities allow only the commands and plugins we use,
  and the opener is limited to `https:` URLs.
- Session names are validated (`^[A-Za-z0-9._-]{1,48}$`), file paths are
  confined to the runtime's contents API, and URL import refuses private or
  loopback targets (SSRF guard) and bodies over 20 MB.
- Every user value placed in generated Python is quoted as a Python string literal.
- No telemetry and no analytics. Network egress goes only to Google and GitHub.
- Release artefacts are signed where certificates are configured. The updater,
  once the maintainer enables it (`docs/RELEASING.md`), verifies minisign signatures.
- Disclaimer: Colab's endpoints are internal APIs used by Google's own
  clients. They can change without notice, and the app is not affiliated with Google.

## 7. Testing strategy

| Layer            | Tooling                                           | What it proves                                                                                                                                                                                                                                       |
| ---------------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Core unit        | `cargo test`                                      | pure logic: XSSI, URL building, shape/accelerator resolution, quota maths, history export, run-file/job parsing, automation code generation, param validation, PKCE                                                                                  |
| Core integration | `cargo test` + `nzap-mock-colab`                  | the real client code against mock Google OAuth, Colab control plane, Jupyter proxy, kernel WebSocket (stdin, `colab_request` consent pause/resume, SystemExit), `/colab/tty` and GitHub catalog. This is the Rust port of colab-studio's 130+ checks |
| Tauri commands   | `cargo test` (tauri `test` runtime)               | command wiring, error mapping, channel streaming                                                                                                                                                                                                     |
| Frontend unit    | Vitest + Testing Library                          | IPC layer, hooks, output renderer, param form, quota chip, panels                                                                                                                                                                                    |
| Web E2E          | Playwright (Chromium + WebKit)                    | full UI flows against an in-browser fake engine (`mockIPC`), on every OS                                                                                                                                                                             |
| Desktop E2E      | WebdriverIO + `tauri-driver` (Linux + Windows CI) | the **real binary** and real Rust core, pointed at `mock-colab`: connect Google, create runtime, run cells, stdin, Drive consent, files, terminal, notebooks, jobs, disconnect                                                                       |

macOS has no WebDriver for WKWebView, so desktop E2E runs on Linux and
Windows while macOS runs all other layers. Real Google/Colab calls are never
made in CI. A manual live-verification checklist ships in `docs/TESTING.md`.

## 8. CI/CD

- `ci.yml` (push and PR): frontend lint, format, typecheck, Vitest and build;
  Rust fmt, clippy (`-D warnings`) and tests on ubuntu, windows and macos;
  Playwright web E2E; desktop E2E on ubuntu (xvfb) and windows; `cargo deny` /
  `npm audit`.
- `release.yml` (tag `v*`): `tauri-action` matrix producing Windows `.msi` and
  NSIS `.exe`, macOS universal `.dmg`, and Linux `.AppImage`, `.deb` and `.rpm`, attached to
  a draft GitHub Release with `latest.json` for the updater. Signing and notarisation run
  only when the matching secrets exist.
- Dependabot for cargo, npm and actions.

## 9. Phases

Each phase ends green in CI and with a commit and push to `main`.

### Phase 0 — Plan & foundation

- [x] This plan, README, LICENSE (Apache-2.0), CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, templates
- [x] Vite + React 19 + TS strict + Tailwind v4 (NZAP tokens) + ESLint + Prettier
- [x] Cargo workspace: `nzap-core`, `nzap-mock-colab`, `src-tauri` (Tauri 2) skeletons
- [x] CI: frontend checks + Rust fmt/clippy/test on three OSes (green on all three OSes)

### Phase 1 — Core foundations: config, HTTP, OAuth, secrets

- [x] Constants ported 1:1 (`config.py`): domains, headers, scopes, accelerators, shapes
- [x] Error model and IPC error codes
- [x] Colab HTTP layer: XSSI strip, `authuser=0`, client-agent header, timeouts
- [x] OAuth: PKCE, loopback listener, remote copy/paste flow, refresh, userinfo, BYO client
- [x] Secret store: keychain with file fallback; app paths injected by the shell
- [x] Mock OAuth server; unit + integration tests

### Phase 2 — Colab control plane & runtime proxy

- [x] `ColabClient`: assign (GET token → POST), unassign, assignments, keep-alive, user-info, ccu-info, runtime specs, resources, credential propagation (the CLI's `/tun/m` route; the unused `v1` route is not ported)
- [x] Quota summary (burn rate, free minutes, severity, tooltip, signup action)
- [x] `RuntimeProxy`: contents (list/read/download/write/upload/mkdir/delete/rename), kernels, sessions
- [x] `nzap-mock-colab` HTTP surface; integration tests

### Phase 3 — Kernel, sessions, history, terminal

- [x] Kernel WebSocket channel: multiplexed execute, stdin replies, `colab_request` handling, execution state
- [x] Session manager: create/adopt/connect/restart/interrupt/shutdown/stop/release, persistence, keep-alive tasks (60 s, 24 h cap), Drive/GCP consent pause & resume
- [x] History log + ipynb/md/txt/jsonl export
- [x] Terminal bridge (`/colab/tty`, header auth only, frame validation)
- [x] Mock kernel + TTY servers; integration tests

### Phase 4 — Automations, run-file, import, jobs, notebooks

- [x] Automations: install (uv → pip), drivemount, gcp-auth
- [x] Run a `.py`/`.ipynb` with env and stop-on-error; executed-notebook output
- [x] Import from Colab/Drive/GitHub/HTTPS with the SSRF guard and size cap
- [x] Ephemeral jobs: assign → run → artifacts → release (always released)
- [x] Notebooks: GitHub catalog (index + SHA-256 + ETag cache + bundled fallback), local private store, fork, export, param validation + `params` injection
- [x] `nzap-notebooks` repo content (catalog, schema, CI validator, contributing guide) — prepared locally; the GitHub repository still has to be created

### Phase 5 — Tauri shell

- [x] Commands + channels for the whole parity map, with typed errors
- [x] Plugins: opener (https only, used from Rust), dialog, log, single-instance, window-state
- [x] Capabilities, CSP, window config (native drag and drop off so HTML5 drop zones work)
- [x] Native save dialogs for downloads, exports and artifacts (uploads use HTML file inputs and drag and drop; job artifacts land in the artifacts folder)
- [x] Command-layer tests

### Phase 6 — Frontend port

- [x] Design system, shell and sidebar from legacy-nzap (Colab-first navigation)
- [x] IPC data layer that keeps legacy hook names and types
- [x] Colab workspace: connection card, consumption chip, runtimes, console, setup, history, terminal, run/jobs, files, notebooks (public + yours)
- [x] Account (Google identity, connection, Colab plan and compute units, privacy — replaces profile/account/credits) and Settings (keep-alive, catalog, artifacts folder, OAuth client, diagnostics)
- [x] Onboarding: first launch → "Connect Google"
- [x] Vitest suites, driven through a simulated engine (`src/dev/fake-engine.ts`) that also powers `npm run dev` in a browser

### Phase 7 — End-to-end tests ✅

- [x] Web E2E (Playwright + simulated engine) covering every workspace flow, plus keyboard access, in Chromium and WebKit
- [x] Desktop E2E (tauri-driver + mock-colab), with an OAuth round-trip driven through the loopback
- [x] Wired into CI on Linux + Windows. It caught two release blockers that no other suite could see: `freezePrototype` crashed the UI in the real webview, and WebView2's automation flags were shadowed

### Phase 8 — Packaging & release ✅

- [x] App icons, bundle metadata, installers for all targets (NSIS per-user + MSI, universal DMG, AppImage/deb/rpm)
- [x] `Cargo.lock` committed; CI builds with `--locked`
- [x] `release.yml` with `tauri-action` (draft releases on tags, build-only runs when packaging changes), optional signing/notarisation. The updater is documented in `docs/RELEASING.md` and needs a maintainer-generated key before it can be enabled
- [x] Install docs per OS (`docs/INSTALL.md`)

### Phase 9 — Hardening & docs ✅

- [x] Security review against the threat model (IPC surface, local paths, logging, token handling); `cargo audit` + `npm audit` gates in CI; race-free state file writes
- [x] Close-to-tray with runtimes kept alive; graceful shutdown
- [x] Accessibility and keyboard pass (WAI-ARIA tabs, labelled controls, keyboard E2E)
- [x] Docs: `ARCHITECTURE.md`, `OAUTH.md` (BYO client), `NOTEBOOKS.md`, `TESTING.md` (with the live-verification checklist), `TROUBLESHOOTING.md`, `RELEASING.md`

### Phase 10 — Apps

- [x] `nzap-app/1`: an optional `app.json` per notebook (widgets, runtime, estimates, outputs) and the `application/vnd.nzap.app+json` event protocol, specified in the catalog's APPS.md and validated by its CI
- [x] Engine: the app spec passes through the catalog, local notebooks, forks and `.nzap.json` export/import; unknown formats degrade to plain notebooks
- [x] UI: Apps gallery and app pages — runtime choice or one-click start of the recommended runtime, form widgets, file inputs uploaded to the runtime, live phases against estimates, warm runtimes, measured timings, audio (waveform + captions) / image / video / table / text / file outputs
- [x] Apps tested on real Colab: Kokoro TTS (CPU and T4), Breeze TTS 2 (T4), sentiment (CPU)
- [x] Simulated engine runs apps (stages, warm runs, synthesized WAV outputs); Vitest + Playwright coverage

### Phase 11 — Brand

- [x] NZAP Labs "NZ" chrome ribbon as the app icon (macOS/Windows/Linux sets regenerated from `brand/`), favicon, sidebar lockup with the logo's own "NZΛP" lettering
- [x] Monochrome chrome palette: ink accent inverted per theme, brushed-metal primary actions, chrome display type; mint/coral kept for status
- [x] `brand/build_brand.py` rebuilds every asset (icon, marks, wordmark masks, social card) from the two source logos

### Phase 12 — Updates & first release

- [x] Updater + process plugins, signing key generated (private key with the maintainer), feed: public releases repository first, this repository once public
- [x] Settings → Updates (check, notes, download progress, install and relaunch) and a quiet check after launch
- [x] `release.yml`: signed updater bundles + `latest.json` when the key secret exists, optional publishing to the public releases repository, Linux-only build-only runs on branches
- [x] `lockfile.yml`: Cargo.lock for dependency changes without a local Rust toolchain
- [ ] v0.1.0 tagged and published (needs the secrets above to ship an update feed)

## 10. Risks & open items

| Risk                                                                                                                 | Mitigation                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Colab internal endpoints change                                                                                      | All endpoints live in one module, mock-based contract tests document the expected shapes, and errors surface with the raw status |
| Default OAuth client (the Google Cloud SDK installed-app client that colab-cli reuses) could be restricted by Google | BYO OAuth client in Settings, `NZAP_OAUTH_CLIENT_JSON`, or `oauth-client.json`; documented in `docs/OAUTH.md`                    |
| `nzap-labs/nzap-notebooks` does not exist yet                                                                        | The app ships a bundled catalog snapshot; repo content is prepared locally for the maintainer to push                            |
| No keychain on minimal Linux                                                                                         | 0600 file fallback with a warning in Account                                                                                     |
| Code-signing certificates are not available yet                                                                      | Unsigned builds still work; signing steps activate only when the secrets are set                                                 |
| No local Rust toolchain on the maintainer machine                                                                    | CI on all three OSes is the build gate                                                                                           |
