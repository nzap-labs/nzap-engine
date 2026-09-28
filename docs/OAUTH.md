# Google sign-in and OAuth clients

NZAP Engine has no account system of its own. **Connect Google** runs a
standard OAuth 2.0 installed-app flow on your machine, and everything
afterwards talks to Google directly with your token.

## What happens when you connect

1. The app generates a PKCE verifier (RFC 7636, `S256`) and starts a one-shot
   listener on `127.0.0.1` and `::1` on a random port.
2. Your browser opens Google's consent page with
   `redirect_uri=http://localhost:<port>/callback`.
3. After you approve, Google redirects to that listener. It accepts exactly
   one request that carries the expected `state`, answers with a short
   "Google connected" page, and closes. It gives up after five minutes.
4. The app exchanges the code for tokens, stores the **refresh token** in the
   OS keychain, and keeps the access token in memory only.

**Use a code** is the fallback when the browser cannot reach the app (remote
desktops, sandboxed browsers). Google shows a code on
`sdk.cloud.google.com/applicationdefaultauthcode.html`, and you paste it into
the app.

**Disconnect** revokes the token at Google, then deletes it locally.

### Scopes

All requested at once, the same set Google's Colab CLI and VS Code extension use:

| Scope                                            | Why                                                            |
| ------------------------------------------------ | -------------------------------------------------------------- |
| `openid`, `userinfo.email`, `userinfo.profile`   | Show who is connected (Account page and sidebar)               |
| `https://www.googleapis.com/auth/colaboratory`   | Allocate and drive Colab runtimes                              |
| `https://www.googleapis.com/auth/drive.file`     | `drive.mount()` in a runtime, after you confirm it per runtime |
| `https://www.googleapis.com/auth/cloud-platform` | `google.colab.auth.authenticate_user()` in a runtime           |

Nothing is sent to a runtime until code running there asks for it and you
approve the prompt in the app.

## The built-in client

By default the app signs in with the installed-app OAuth client that Google
ships with the Cloud SDK. Google's own
Colab CLI (`google-colab-cli`) uses the same client.
Installed-app client secrets are not confidential (Google's
[documentation](https://developers.google.com/identity/protocols/oauth2/native-app)
says so), which is why it can be embedded in open-source code.

Google could restrict that client at any time. If sign-in starts failing with
`invalid_client` or `unauthorized_client`, use your own.

## Bring your own client

1. In the [Google Cloud console](https://console.cloud.google.com/apis/credentials),
   create or pick a project.
2. **OAuth consent screen**: user type _External_ (or _Internal_ in a
   Workspace). Add your account as a test user while the app is in testing.
3. **Credentials → Create credentials → OAuth client ID → Desktop app.**
   Download the JSON.
4. Give it to NZAP Engine, in any of these ways (checked in this order):
   - the environment variable `NZAP_OAUTH_CLIENT_JSON` containing the JSON
     text (useful for managed installs);
   - **Settings → Google OAuth client**: paste the JSON and save (this writes
     `oauth-client.json` in the config directory);
   - put the file yourself at `<config dir>/oauth-client.json` (see
     [INSTALL.md](./INSTALL.md#where-things-are-stored)).
5. Disconnect Google if you were connected, then connect again.

Desktop-app clients accept any `http://localhost:<port>` redirect, so there is
nothing else to register. The **Use a code** flow only works with clients that
have Google's copy/paste landing page registered, which in practice means the
built-in one; with your own client, use the browser flow.

**Settings → Google OAuth client → Built-in** switches back.

## Where the token lives

| System  | Store                                                                  |
| ------- | ---------------------------------------------------------------------- |
| Windows | Credential Manager, service `com.nzaplabs.engine`                      |
| macOS   | Login keychain, service `com.nzaplabs.engine`                          |
| Linux   | Secret Service (GNOME Keyring, KWallet), service `com.nzaplabs.engine` |

When no keychain is available (minimal Linux installs, some containers), the
refresh token goes to `secrets.json` in the config directory with `0600`
permissions, and the Account page says so.
