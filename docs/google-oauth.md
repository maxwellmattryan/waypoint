# Google account (Gmail and Calendar)

Waypoint reads your Gmail and creates Calendar events through one Google account that you connect once. It asks for exactly two scopes:

- `https://www.googleapis.com/auth/gmail.readonly`: read your Gmail ("Read your Gmail"). Waypoint can't send, delete or change mail.
- `https://www.googleapis.com/auth/calendar.events`: view and edit events on your calendars ("View and edit events on all your calendars"). It can't change calendar settings or sharing.

The sign-in is an OAuth authorization-code flow with PKCE and a single-use `state`, redirected to the API on `127.0.0.1`. The refresh and access tokens are stored in Postgres encrypted with XChaCha20-Poly1305 under `WAYPOINT_TOKEN_KEY`, never in plaintext, and never written to logs.

Google needs an OAuth client that belongs to you. Nobody else's client can be used, so you create one in your own Google Cloud project, once.

## 1. Create the Google Cloud project

1. Open the [Google Cloud console](https://console.cloud.google.com/) and create a project, e.g. `waypoint`.
2. In **APIs & Services → Library**, enable the **Gmail API** and the **Google Calendar API**.

## 2. Configure the OAuth consent screen

1. Open **APIs & Services → OAuth consent screen** (in newer consoles: **Google Auth Platform → Branding / Audience / Data access**).
2. Choose **External** as the user type.
3. Set the app name (e.g. `Waypoint`), your email as the user support email and as the developer contact.
4. Under **Scopes** (**Data access**), add exactly these two and nothing else:
   - `.../auth/gmail.readonly`
   - `.../auth/calendar.events`

## 3. Publish the app "In production", unverified

On the consent screen (**Audience**), click **Publish app** so the publishing status is **In production**. Don't submit it for verification.

Why: an app left in **Testing** gets refresh tokens that Google expires after 7 days, so Waypoint would ask you to reconnect every week. An unverified app in production keeps its refresh token, and it works for personal use (up to 100 users). The price is Google's "Google hasn't verified this app" warning on the consent page. It's your own app, so click **Advanced → Go to Waypoint (unsafe)** to continue.

## 4. Create the OAuth client

1. Open **APIs & Services → Credentials** (**Google Auth Platform → Clients**) and choose **Create credentials → OAuth client ID**.
2. Pick the application type **Desktop app** and name it, e.g. `Waypoint local`.
3. Copy the client ID and the client secret now, or download the JSON. The console may not show the secret again.

Desktop-app clients accept a loopback redirect on any port, so you don't register a redirect URI. Waypoint uses `http://127.0.0.1:<WAYPOINT_API_PORT>/auth/google/callback`.

## 5. Add the settings to `.env`

Add the client and a fresh token key to `.env` (never to `.env.example`, which is committed):

```sh
WAYPOINT_GOOGLE_CLIENT_ID=<client id>.apps.googleusercontent.com
WAYPOINT_GOOGLE_CLIENT_SECRET=<client secret>
WAYPOINT_TOKEN_KEY=<output of: openssl rand -base64 32>
```

`WAYPOINT_TOKEN_KEY` must be 32 random bytes in standard base64; the API refuses to start with any other value. Keep it secret and never commit it (`.env` is gitignored). If you lose or change it, the stored tokens no longer decrypt: the next time Waypoint uses them, it marks the account `reconnect_required` and you connect again.

Until all three settings are set, the rest of Waypoint works and Google reports "not configured".

## 6. Connect

1. Restart the API (`./scripts/dev.sh`).
2. Open <http://127.0.0.1:8000/auth/google/connect> in your browser. It redirects to Google.
3. Click through the unverified-app warning, check that the consent page lists only "Read your Gmail" and "View and edit events on all your calendars", leave both ticked and approve.
4. The browser lands on "Google connected. You can close this tab."
5. Check the status:

   ```sh
   curl -s http://127.0.0.1:8000/auth/google
   # {"configured":true,"state":"connected","scopes":[...],"connected_at":"..."}
   ```

If you untick a permission, Waypoint revokes the partial grant, stores nothing and asks you to start again. A sign-in link works once and expires after 10 minutes.

`/auth/google/connect` answers 403 when another website tries to start the flow (`Sec-Fetch-Site: cross-site`), so only you, typing or clicking the URL, can start it.

## Status and reconnecting

`GET /auth/google` reports `state`:

- `disconnected`: nothing is connected.
- `connected`: the tokens are stored and refresh themselves.
- `reconnect_required`: Google revoked or expired the grant, or the tokens no longer decrypt (the key changed). Open `/auth/google/connect` again.

Waypoint finds this out only when it next uses the tokens, so right after a revocation or a key change the status still reads `connected`.

## Disconnect

```sh
curl -s -X DELETE http://127.0.0.1:8000/auth/google
# {"revoked":true}
```

Like connect, disconnect answers 403 to a request from another website (`Sec-Fetch-Site: cross-site`) and to an HTML form post, so a page you visit can't disconnect you.

Waypoint asks Google to revoke the grant and deletes the stored tokens either way. When the answer is `{"revoked":false}` Google didn't confirm, so remove Waypoint by hand at <https://myaccount.google.com/permissions>.
