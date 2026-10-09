# Security Policy

## Reporting a vulnerability

Please report vulnerabilities **privately** through
[GitHub Security Advisories](https://github.com/nzap-labs/nzap-engine/security/advisories/new).
Do not open a public issue. We aim to acknowledge reports within 3 working days
and to ship a fix or mitigation for confirmed issues as quickly as their severity requires.

Only the latest release is supported with security fixes.

## What NZAP Engine protects

NZAP Engine acts with the Google account you connect. The assets it guards are:

| Asset                      | Where it lives                                                                  | Protection                                                 |
| -------------------------- | ------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Google refresh token       | OS keychain (a 0600 file only where no keychain exists, with a visible warning) | never sent to the webview, never logged                    |
| Google access token        | process memory                                                                  | short-lived, refreshed ahead of expiry, never logged       |
| Runtime proxy tokens       | `sessions.json` in the app data directory (0600)                                | VM-scoped and short-lived; never sent to the webview       |
| Your notebooks and history | app data directory                                                              | local only; nothing is uploaded except to your own runtime |

## Design decisions

- **No listening server.** The UI talks to the engine over Tauri IPC, restricted
  by capabilities. The only socket is the OAuth loopback listener: bound to
  loopback, ephemeral port, one request, closed after use or after 5 minutes.
- **AI agents over stdio.** `nzap-engine mcp` serves MCP on stdin/stdout to
  the client that launched it, never on a port. Agents never receive Google
  tokens. They may read and write local files only inside the folder the server
  started in and any `--allow-dir` (checked lexically and after resolving
  symlinks). Each agent holds at most `--max-runtimes` VMs, and the ones it
  started are released when it disconnects. Running code on your VMs and
  spending compute units is what the tools are for, so leave your MCP client's
  per-call approval on unless you trust the agent. See [docs/MCP.md](./docs/MCP.md).
- **OAuth with PKCE (S256)** and a single-use `state`.
- **Strict CSP.** No remote scripts. The external opener only accepts `https:` URLs.
- **Input confinement.** Session names are validated. File operations go through
  the runtime's Jupyter contents API only. URL import rejects private, loopback
  and link-local targets and caps downloads at 20 MB. User values embedded in
  generated Python are always Python string literals.
- **Public notebooks** are fetched from GitHub over HTTPS and verified against
  the SHA-256 digests in the catalog. They run on your Colab VM, never on your
  machine, and you can read the source before running one.
- **No telemetry.** Network traffic goes only to Google (accounts, Colab, Drive,
  your runtimes) and GitHub (the notebook catalog).
- **Supply chain.** `Cargo.lock` and `package-lock.json` are committed and CI
  builds with `--locked`. Every push runs `cargo audit` (RustSec) and
  `npm audit` on the shipped dependencies.

## Known limits

- Colab's endpoints are internal APIs used by Google's own clients. They may
  change, and NZAP Engine cannot guarantee their behaviour.
- Anyone with access to your unlocked OS user account can use the stored Google
  connection, as with any desktop app that remembers a sign-in. That includes
  any program that launches `nzap-engine mcp`. Use
  **Account → Disconnect** to revoke it locally, and
  [Google account permissions](https://myaccount.google.com/permissions) to revoke it everywhere.
