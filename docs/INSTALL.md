# Installing NZAP Engine

Download the installer for your system from the
[releases page](https://github.com/nzap-labs/nzap-engine/releases). There is
nothing else to install: no Python, no Node, no Docker and no server.

## Windows 10 / 11

- **`NZAP.Engine_x.y.z_x64-setup.exe`** (recommended): installs for the
  current user, no administrator rights needed.
- **`NZAP.Engine_x.y.z_x64_en-US.msi`**: for managed deployments.

The app uses Microsoft Edge WebView2, which Windows 11 already has; the
installer fetches it on older Windows 10 systems.

If a release is not code-signed yet, SmartScreen shows _"Windows protected your
PC"_. Choose **More info → Run anyway**.

## macOS 10.15 or later

Open **`NZAP.Engine_x.y.z_universal.dmg`** and drag the app to Applications.
The build runs natively on Apple silicon and Intel.

If a release is not notarised yet, macOS refuses to open it the first time.
Right-click the app → **Open** → **Open**, or run:

```bash
xattr -dr com.apple.quarantine "/Applications/NZAP Engine.app"
```

## Linux

- **AppImage** (any distribution): `chmod +x NZAP.Engine_*.AppImage` and run it.
- **Debian / Ubuntu**: `sudo apt install ./nzap-engine_*_amd64.deb`
- **Fedora / RHEL / openSUSE**: `sudo dnf install ./nzap-engine-*.x86_64.rpm`

The app needs WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`), which the `.deb` and
`.rpm` packages pull in. For the Google token it uses the Secret Service
(GNOME Keyring, KWallet); without one it falls back to a file only your user
can read, and the Account page says so.

## First launch

1. Click **Connect Google**. Your browser opens Google's consent page; approve
   it and come back — the app finishes the sign-in by itself.
2. If the browser cannot reach the app (for example on a remote desktop), use
   **Use a code** instead and paste the code Google shows.
3. Launch a runtime from the **Runtimes** tab.

## Where things are stored

| What                              | Windows                                   | macOS                                               | Linux                                     |
| --------------------------------- | ----------------------------------------- | --------------------------------------------------- | ----------------------------------------- |
| Runtimes, history, your notebooks | `%APPDATA%\com.nzaplabs.engine`           | `~/Library/Application Support/com.nzaplabs.engine` | `~/.local/share/com.nzaplabs.engine`      |
| Settings, Google identity         | `%APPDATA%\com.nzaplabs.engine`           | `~/Library/Application Support/com.nzaplabs.engine` | `~/.config/com.nzaplabs.engine`           |
| Logs                              | `%LOCALAPPDATA%\com.nzaplabs.engine\logs` | `~/Library/Logs/com.nzaplabs.engine`                | `~/.local/share/com.nzaplabs.engine/logs` |
| Google refresh token              | Windows Credential Manager                | Keychain                                            | Secret Service                            |

**Settings → About → Open log folder** opens the logs directly.

## Uninstalling

Use the system's usual uninstaller. To also forget your Google connection,
first click **Disconnect** on the Account page (it revokes the app's access at
Google), or remove it at
[myaccount.google.com/permissions](https://myaccount.google.com/permissions).
