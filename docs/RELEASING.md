# Releasing

For maintainers. Releases are built by
[`.github/workflows/release.yml`](../.github/workflows/release.yml) on GitHub's
runners: a universal macOS build, Linux (`.AppImage`, `.deb`, `.rpm`), and
Windows (NSIS setup `.exe` and `.msi`).

## Cutting a release

1. Make sure `main` is green, and run the
   [live checklist](./TESTING.md#live-verification-checklist).
2. Bump the version in `package.json`, `Cargo.toml` (`[workspace.package]`) and
   `src-tauri/tauri.conf.json`. They must match.
3. Commit, then tag and push:
   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```
4. The workflow creates a **draft** release with the installers attached.
   Check the assets, write the notes, and publish it. A tag with a hyphen
   (`v0.2.0-beta.1`) is marked as a pre-release.

## Code signing (optional)

Unsigned builds work. Users see a SmartScreen or Gatekeeper warning, and
[INSTALL.md](./INSTALL.md) explains how to get past it. Signing turns on by
itself when the repository has these secrets:

**macOS signing and notarisation**

| Secret                       | Value                                                    |
| ---------------------------- | -------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | base64 of the exported _Developer ID Application_ `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password                                      |
| `APPLE_SIGNING_IDENTITY`     | e.g. `Developer ID Application: NZAP Labs (TEAMID)`      |
| `APPLE_ID`                   | the Apple ID that notarises                              |
| `APPLE_PASSWORD`             | an app-specific password for that Apple ID               |
| `APPLE_TEAM_ID`              | the team ID                                              |

**Windows**: signing needs a code-signing certificate, from a CA or through
Azure Trusted Signing. Configure it in `bundle.windows` following
[Tauri's Windows signing guide](https://v2.tauri.app/distribute/sign/windows/),
and add the secrets to the release workflow.

## Auto-update (not enabled yet)

The Tauri updater verifies every update against a public key compiled into the
app. Only the maintainer should hold the private key, so it is not generated
here. To turn updates on:

1. Generate the key pair once, on a trusted machine, and keep the private key
   and its password safe. Losing it means users must reinstall by hand.
   ```bash
   npx tauri signer generate -w ~/.tauri/nzap-engine.key
   ```
2. Add the repository secrets `TAURI_SIGNING_PRIVATE_KEY` (the file contents)
   and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and pass both as `env` to the
   `tauri-action` step in `release.yml`.
3. Add the plugin: `tauri-plugin-updater` in `src-tauri/Cargo.toml`,
   `.plugin(tauri_plugin_updater::Builder::new().build())` in
   `src-tauri/src/lib.rs`, and `"updater:default"` in
   `src-tauri/capabilities/default.json`.
4. In `src-tauri/tauri.conf.json`, set `bundle.createUpdaterArtifacts` to
   `true` and add:
   ```json
   "plugins": {
     "updater": {
       "pubkey": "<contents of nzap-engine.key.pub>",
       "endpoints": [
         "https://github.com/nzap-labs/nzap-engine/releases/latest/download/latest.json"
       ]
     }
   }
   ```
5. Set `updaterJsonPreferNsis: true` on the `tauri-action` step so Windows
   updates use the per-user installer. The action then uploads `latest.json`
   with each release.
6. Add an update check to the UI (Settings → About), using
   `@tauri-apps/plugin-updater`.

## Dependencies

`Cargo.lock` and `package-lock.json` are committed. Release builds use exactly
those versions. Dependabot proposes updates weekly, and CI must pass before
merging.
