# Troubleshooting

Start with the logs: **Settings → About → Open log folder**. The paths are in
[INSTALL.md](./INSTALL.md#where-things-are-stored). Logs never contain tokens.

## Sign-in

**The browser says "This site can't be reached" after I approve.**
Something blocked the app's one-time listener on `localhost` (a firewall, a
browser on another machine, or a sandboxed browser). Cancel, and use
**Use a code** instead.

**"Access blocked" or `invalid_client` / `unauthorized_client` from Google.**
Google may have restricted the built-in OAuth client. Use your own; see
[OAUTH.md](./OAUTH.md#bring-your-own-client).

**"Your Google connection was revoked. Connect again."**
The refresh token no longer works (you removed the app at Google, changed your
password, or the token went unused for six months). Connect again.

**"No system keychain was available, so the token is stored in a file…" (Linux).**
No Secret Service is running. Install and unlock GNOME Keyring or KWallet,
then disconnect and connect again to move the token into it.

## Runtimes

**"This account already holds the maximum number of Colab VMs."**
Colab limits concurrent runtimes per account. Stop one in **Runtimes**, or
release one created elsewhere (a browser tab, the Colab CLI) under
_External runtimes_.

**"Colab rejected accelerator …" / an accelerator is marked "not available on your plan".**
Free accounts get GPUs only when Colab has capacity, and paid plans spend
compute units. Try a CPU runtime or a different accelerator, or check units on
the Account page.

**A runtime stopped on its own.**
Colab stops idle runtimes, and every runtime after at most 12–24 hours
depending on the plan. The keep-alive in Settings prevents idle stops while
the app is running (up to 24 hours). With **close-to-tray**, closing the window
does not stop it.

**A cell hangs on `drive.mount()`.**
The app is waiting for your consent; look for the prompt in the console. If
you declined, run the cell again.

## The app

**Linux: the window is blank.**
Some GPU drivers break WebKitGTK's DMA-BUF renderer. Start the app with
`WEBKIT_DISABLE_DMABUF_RENDERER=1`.

**Linux: no tray icon with close-to-tray on.**
GNOME needs the _AppIndicator and KStatusNotifierItem Support_ extension.
Without one the icon is invisible; launching NZAP Engine again brings the
hidden window back.

**Windows: the app does not start on an old Windows 10.**
Install the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/).

**Public notebooks don't load.**
The app needs `raw.githubusercontent.com`. Offline, it shows the last
downloaded collection, or the snapshot bundled with the app. If you changed
**Settings → Public notebook collection**, check that the URL ends at the
folder containing `index.json`.

## Reporting a bug

Open an issue with the app version (Settings → About), your OS, what you did,
and the relevant log lines. Please don't include your email address or the
contents of private notebooks. For security problems, see
[SECURITY.md](../SECURITY.md) instead.
