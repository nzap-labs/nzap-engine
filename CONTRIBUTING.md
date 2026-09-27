# Contributing to NZAP Engine

Thanks for helping. This guide keeps changes easy to review and safe to ship.

## Ground rules

- **Be kind.** See the [Code of Conduct](./CODE_OF_CONDUCT.md).
- **Security issues are private.** Follow [SECURITY.md](./SECURITY.md) and do not open a public issue.
- **Never commit secrets.** No tokens, runtime-proxy URLs, OAuth client secrets
  of your own, or personal data, including in tests and fixtures.

## Getting set up

1. Install Node 22+, Rust stable and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
2. Run `npm install`.
3. Run `npm run app:dev`.

## Before you open a PR

```bash
npm run lint && npm run format:check && npm run typecheck && npm test
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks on Linux, macOS and Windows.

## Where things go

| Change                                  | Location                                                                               |
| --------------------------------------- | -------------------------------------------------------------------------------------- |
| Talking to Google / Colab / the runtime | `crates/nzap-core` with a test against `crates/nzap-mock-colab`                        |
| New IPC command                         | `src-tauri/src/commands/`, mirrored in `src/api/` and `src/types/`                     |
| UI                                      | `src/features/`, using the NZAP tokens in `src/styles/app.css`                         |
| Public notebooks                        | the [nzap-notebooks](https://github.com/nzap-labs/nzap-notebooks) repository, not here |

## Colab wire behaviour

The engine follows Google's own clients. When you add or change a Colab endpoint,
header or value, cite where it comes from (`google-colab-cli` or `colab-vscode`)
in a code comment. Add a mock-backed test for it, and describe in the PR how you
verified it against a real runtime.

## Commits

Use short imperative subjects with a scope, for example
`feat(core): assign TPU runtimes` or `fix(ui): keep console scroll position`.
