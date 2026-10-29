# Ranch server

The same UI, served over HTTP by the same Rust core, for the places a desktop app does not go: next to a staging Redis, in a compose stack, in a cluster.

```bash
# Once: the key that encrypts the passwords Ranch stores. Keep this file.
(umask 077 && openssl rand -base64 32 > ranch-secret.key)

docker run -d --name ranch -p 127.0.0.1:8080:8080 -v ranch-data:/data \
  -e RANCH_PASSWORD='choose-one' -e RANCH_SECRET_KEY="$(cat ranch-secret.key)" \
  ghcr.io/bramvanosta/ranch:latest
```

Open http://localhost:8080 (user `ranch`). The key is generated once, not at every `docker run`: the passwords Ranch stores are encrypted with it, and a container recreated with another key cannot read them. `:latest` is the newest release; pin a version (`:0.4.0`) to choose when you upgrade, or follow `:rc` (pre-releases) or `:main`. The image runs on amd64 and arm64.

To build the image from source instead:

```bash
docker build -t ranch .
docker run -d --name ranch -p 127.0.0.1:8080:8080 -v ranch-data:/data \
  -e RANCH_PASSWORD='choose-one' -e RANCH_SECRET_KEY="$(cat ranch-secret.key)" ranch
```

Or, with a demo Redis beside it:

```bash
RANCH_PASSWORD=choose-one docker compose -f docker/compose.yaml up --build
```

| Variable | Default | |
|---|---|---|
| `RANCH_PASSWORD` | - | Password of the single account (HTTP Basic). **Either this or OIDC is required**: without authentication the server refuses to start. |
| `RANCH_USERNAME` | `ranch` | Its user name. |
| `RANCH_OIDC_*` | - | Sign-in through your identity provider (see below). |
| `RANCH_SECRET_KEY` | unset | Encrypts the Redis / SSH passwords Ranch stores in `/data` (XChaCha20-Poly1305). Unset: stored in clear, and the server says so at start-up. Use a random value (`openssl rand -base64 32`): it is hashed, not stretched, so a short one can be guessed offline; the server warns under 32 characters. Keep it: a stored secret cannot be read with another key. |
| `RANCH_ALLOW_NO_AUTH` | unset | `1` = no password. Only on a network you trust end to end. |
| `RANCH_PORT` / `RANCH_BIND` | `8080` / `0.0.0.0` | |
| `RANCH_DATA_DIR` | `/data` | SQLite database (connections, groups, audit log, settings) and the secrets file. **Mount a volume.** A bind mount must be writable by uid 65532. |

<details>
<summary>Sign-in with OIDC (Pocket ID, Authentik, Keycloak…)</summary>

Register Ranch at the provider as a confidential client with the callback `https://<where Ranch lives>/auth/callback`, then:

```yaml
environment:
  RANCH_OIDC_ISSUER: https://id.example.com          # its /.well-known/openid-configuration is fetched
  RANCH_OIDC_CLIENT_ID: …
  RANCH_OIDC_CLIENT_SECRET: …
  RANCH_PUBLIC_URL: https://ranch.example.com         # the callback is <this>/auth/callback
  RANCH_SECRET_KEY: …                                 # also keeps sessions across restarts
  # RANCH_OIDC_ALLOWED_EMAILS: me@example.com,you@example.com
  # RANCH_OIDC_ALLOWED_GROUPS: admins                 # `groups` claim; either list matching is enough
```

| Variable | Default | |
|---|---|---|
| `RANCH_OIDC_ISSUER` | - | Turns OIDC on. The three below are then required, and a missing one is named at start-up. |
| `RANCH_OIDC_CLIENT_ID`, `RANCH_OIDC_CLIENT_SECRET` | - | The client registered at the provider. |
| `RANCH_PUBLIC_URL` | - | Where users reach Ranch. `https://` makes the cookies `Secure`. |
| `RANCH_OIDC_SCOPES` | `openid profile email` | `groups` is added when groups are filtered. |
| `RANCH_OIDC_ALLOWED_EMAILS`, `RANCH_OIDC_ALLOWED_GROUPS` | unset | Who may use Ranch. **Unset = whoever the provider lets into this client** (in Pocket ID, restrict the client to a user group). An e-mail only counts when the provider marks it verified (`email_verified`). |
| `RANCH_SESSION_SECRET` | `RANCH_SECRET_KEY`, else random | Signs the session cookie. Random = everyone signs in again after a restart. |
| `RANCH_SESSION_TTL_HOURS` | `12` | How long a sign-in lasts. |

Authorization code + PKCE; the ID token is verified (signature against the provider's keys, asymmetric algorithms only, issuer, audience, expiry, nonce); the session is a signed, `HttpOnly`, `SameSite=Lax` cookie. When a session ends the page goes back to the provider and returns where it was. The foot of the sidebar shows who is signed in, and *Sign out*. *Sign out* revokes that session on the server: a copy of its cookie opens nothing afterwards, restarts included. To sign everyone out at once, change `RANCH_SESSION_SECRET`. With OIDC on, `RANCH_PASSWORD`, if set, stays accepted as HTTP Basic: a break-glass for the day the provider is down, and a way in for scripts (`curl -u`). Leave it unset for OIDC only.

</details>

**Security model.** One shared account or your identity provider; credentials or the session cookie travel on every request, so **put TLS in front** (Caddy, Traefik, nginx…) before exposing it beyond localhost. The API refuses requests a foreign web page could forge (a required custom header), and everything except `/healthz` is behind the password. Ranch can pause queues and delete jobs: treat its URL like an admin console. Whoever signs in is trusted like that: they can point a connection at any host and port the server reaches (your internal network included), and an SSH key path at any file the container can read, so mount only the keys Ranch needs. A saved password is only ever sent back over the route it was saved for: the same host, port, TLS and CA, through the same SSH bastion and jump host. Wrong passwords are slowed down; past 10 in a minute from one address, HTTP Basic sign-in is refused to that address (`429`, right password included) until the minute has passed, and past 200 in a minute from all addresses together, to everyone. Behind a reverse proxy, every request comes from the proxy's address, so all clients share the per-address limit. Signed-in OIDC sessions are not affected. Production connections are still locked by default, and every write is in the audit log.

**What the image does not do.** No MCP: agents need a human to approve their writes in the desktop app, next to the agent. No self-update (the desktop app has one): pull a newer tag. No live progress bar on bulk actions (they report when done). SSH tunnels work; a key file must be mounted into the container at the path you give Ranch.

The image is distroless (no shell), runs as non-root, and carries a `HEALTHCHECK` (`ranch-server --healthcheck`). Without Docker: `cargo build --release --no-default-features --features server --bin ranch-server` in `src-tauri/`, `pnpm build`, then run the binary with `RANCH_STATIC_DIR=dist`.
