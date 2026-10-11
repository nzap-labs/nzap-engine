# Runtime templates

NZAP Engine runs code on a Jupyter Server in every provider's VM. Colab VMs
come with one. Codespaces and CodeSandbox VMs get one from these templates.

## GitHub Codespaces (`codespaces/`)

The Codespaces API can create, start, stop and delete a codespace, but it
cannot run commands in one. The codespace's dev container therefore starts
Jupyter itself. It listens on port 8888, which stays private, and NZAP Engine
reaches it at `https://<codespace>-8888.app.github.dev` with the owner's
GitHub token.

To publish it:

```bash
gh repo create nzap-labs/nzap-runtime --public --clone
cp -r runtimes/codespaces/. nzap-runtime/
cd nzap-runtime && git add -A && git commit -m "NZAP runtime" && git push -u origin HEAD
```

A public repository lets anyone create codespaces from it on their own
account, and the codespaces are billed to them. Point **Settings → Compute
providers → Codespaces repository** at a fork to change the image, pre-install
packages, or add a [prebuild](https://docs.github.com/codespaces/prebuilding-your-codespaces)
for faster starts.

## CodeSandbox

No template is needed. NZAP Engine forks CodeSandbox's universal template,
boots the VM, and asks the VM agent to install and start Jupyter (see
`crates/nzap-core/src/providers/codesandbox.rs`). To start faster, build your
own template with Jupyter pre-installed and set its id in **Settings →
Compute providers → CodeSandbox template**.
