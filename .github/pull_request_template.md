## What and why

<!-- One or two sentences. Link the issue if there is one. -->

## How it was tested

- [ ] `npm run lint && npm run typecheck && npm test`
- [ ] `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- [ ] E2E (if UI or IPC changed)
- [ ] Verified against a real Colab runtime (if Colab wire behaviour changed) — describe below

## Checklist

- [ ] No tokens, proxy URLs or personal data in code, tests, fixtures or logs
- [ ] New Colab endpoints/values cite their upstream source (colab-cli / colab-vscode)
- [ ] Docs / PLAN.md updated if behaviour changed
