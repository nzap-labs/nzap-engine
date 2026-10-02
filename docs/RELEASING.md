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

## Auto-update

NZAP Engine updates itself (**Settings → Updates**, and a check a few
seconds after launch). The updater plugin downloads `latest.json` from the
first reachable endpoint in `src-tauri/tauri.conf.json`:

1. `nzap-labs/nzap-engine-releases` — a public, releases-only repository, the
   feed while this repository is private;
2. `nzap-labs/nzap-engine` — the feed once this repository is public.

Every bundle is verified against the public key in `plugins.updater.pubkey`
before it installs. The private key lives only with the maintainer
(`~/.tauri/nzap-engine.key` on the machine that generated it, with its
password beside it). **Back both up**: without them no installed copy can
ever be updated again, and users would have to reinstall by hand.

### One-time setup

1. Repository secrets (Settings → Secrets and variables → Actions):

   | Secret                               | Value                                      |
   | ------------------------------------ | ------------------------------------------ |
   | `TAURI_SIGNING_PRIVATE_KEY`          | the contents of `~/.tauri/nzap-engine.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the contents of `nzap-engine.key.password` |

   With them, `release.yml` builds with `src-tauri/tauri.updater.conf.json`
   (`createUpdaterArtifacts`), signs the bundles and uploads `latest.json`.
   Without them, releases still build; they just cannot update installed apps.

2. While this repository is private: create the **public** repository
   `nzap-labs/nzap-engine-releases` (with a README so it has a default
   branch), then add
   - the variable `RELEASES_REPO` = `nzap-labs/nzap-engine-releases`;
   - the secret `RELEASES_TOKEN`: a fine-grained token with **Contents: read
     and write** on that repository only.

   Tagged releases are then published there. When this repository goes
   public, delete the variable: releases (and `latest.json`) land here, which
   is the second endpoint every installed copy already checks.

Windows installs updates with the NSIS installer in passive mode (a progress
window, no questions); macOS and Linux replace the app and relaunch it.

## Dependencies

`Cargo.lock` and `package-lock.json` are committed. Release builds use exactly
those versions. Dependabot proposes updates weekly, and CI must pass before
merging.
