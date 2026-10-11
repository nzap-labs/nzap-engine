# Compute providers: GitHub Codespaces and CodeSandbox

Besides Google Colab, NZAP Engine can run runtimes on **GitHub Codespaces** (with
your GitHub sign-in) and **CodeSandbox** VM sandboxes (with your API key). Every
feature that works on a runtime works on these too: the console, files, the
terminal, notebooks, apps, ephemeral jobs and AI agents (MCP).

They behave the way Colab runtimes do:

1. **Created on demand.** Launch one from **Runtimes → New runtime**, or an app,
   job or agent asks for one. NZAP Engine creates the VM and waits until it
   serves Jupyter (a few minutes for a new codespace, about a minute for a
   sandbox).
2. **Warm for reuse.** The runtime keeps its kernel, installed packages and
   loaded models until it is deleted. A codespace that GitHub stopped, or a
   sandbox CodeSandbox hibernated, is started again on its next use.
3. **Deleted** when you stop it. They are also deleted automatically after
   **Settings → Compute providers → Delete idle runtimes after** (30 minutes by
   default; 0 = never) without a cell, file or terminal operation. Busy kernels
   are left alone.

These are CPU machines. Use Colab for GPUs and TPUs.

## GitHub Codespaces

**Connect** in **Settings → Compute providers → GitHub Codespaces**:

- **Sign in with GitHub** uses GitHub's device flow. It needs a GitHub OAuth
  app (with device flow enabled) whose client id you enter in the same card, or
  that the build provides as `NZAP_GITHUB_CLIENT_ID`.
- **Use GitHub CLI login** reuses `gh`'s token. It needs the `codespace` scope:
  `gh auth refresh -h github.com -s codespace`.
- **Paste a token**: a classic token with the `codespace` scope, or a
  fine-grained token with the _Codespaces_ read and write permission.

The token is kept in the OS keychain, like the Google connection.

**How it runs.** The Codespaces API can create, start, stop and delete
codespaces, but it cannot run commands in one. So NZAP Engine creates codespaces
from a **runtime repository** whose dev container starts a Jupyter Server on
port 8888 (`runtimes/codespaces/` in this repository is that dev container; see
[runtimes/README.md](../runtimes/README.md)). The port stays **private**: NZAP
Engine reaches `https://<codespace>-8888.app.github.dev` with your token in the
`X-Github-Token` header, and GitHub refuses requests without it.

| Setting            | Default                           |
| ------------------ | --------------------------------- |
| Runtime repository | `nzap-labs/nzap-runtime`          |
| Dev container      | `.devcontainer/devcontainer.json` |
| Default machine    | `basicLinux32gb` (2 cores, 8 GB)  |

New codespaces get GitHub's idle timeout and a **retention period** equal to
the auto-delete delay, so GitHub deletes them itself even when NZAP Engine is
closed. Codespaces usage is billed to your GitHub account (free monthly quota
for personal accounts).

## CodeSandbox

**Connect** in **Settings → Compute providers → CodeSandbox** with an API key
from <https://codesandbox.io/t/api> that has sandbox and VM permissions. The key
is kept in the OS keychain.

**How it runs**, using the same API as CodeSandbox's SDK (`@codesandbox/sdk`):

1. `POST /sandbox/<template>/fork` creates a private sandbox with private
   preview hosts.
2. `POST /vm/<id>/start` boots its VM at the chosen tier.
3. `POST /sandbox/<id>/tokens` creates a preview token.
4. Through the VM agent (the SDK's WebSocket protocol), NZAP Engine installs and
   starts a Jupyter Server on port 8888 with its own random token.
5. Requests go to `https://<id>-8888.csb.app` with the `csb-preview-token`
   header and the Jupyter token.

VMs hibernate when idle and wake on the next request. A VM that booted from
scratch gets Jupyter started again automatically. Stopping a runtime calls
`DELETE /vm/<id>`.

| Setting         | Default                                                               |
| --------------- | --------------------------------------------------------------------- |
| Template        | `pcz35m` (CodeSandbox's universal template)                           |
| Default VM tier | `Nano` (2 CPU, 4 GB). Pico, Nano, Micro, Small, Medium, Large, XLarge |

To start faster, use a template that already has `jupyter_server` and
`ipykernel` installed.

## Notes

- Live resource usage and Drive mounting are Colab features. On other
  providers, use the codespace's or sandbox's own page (**Open in Codespaces /
  CodeSandbox** on the runtime card).
- Disconnecting Google does not touch Codespaces or CodeSandbox runtimes, and
  their runtimes come back after an app restart unless they were deleted
  elsewhere.
- Agents choose a provider with `provider` (`colab`, `codespaces`,
  `codesandbox`) and `machine` on `start_runtime`, `run_job` and `run_app`
  (see [MCP.md](./MCP.md)).
