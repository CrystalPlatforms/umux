# AUTH.md

Identity and access rules for this project. Any AI coding agent working on this
repository follows this file. Human developers follow it too.

Covers: sign-up, login, sessions, passwords, admin roles, OAuth, password reset.
Rate limiting, CSRF, CORS and security headers live in `SECURITY.md`. Database
row access lives in `RLS-POLICIES.md`.

## Hard rules (never break)

1. Authentication happens on the server, on every request. A login screen,
   a hidden menu item, a client-side route guard or middleware alone is UI,
   not security.
2. The user ID used to read or write data comes only from the verified session
   or token. Never from the request body, query string, URL path or a
   client-controlled header.
3. Every server entry point (API route, handler, server function, webhook,
   cron route, RPC, GraphQL resolver) states whether it needs a signed-in
   user, and checks that inside the handler itself.
4. Login and sign-up errors are generic: "Invalid email or password". The
   response must not reveal whether an account exists.
5. Roles come from a source users cannot edit: a roles column in the database
   or server-only provider metadata. Never client-editable user metadata,
   localStorage, a client-set cookie or a request parameter.
6. Never roll your own crypto or auth protocol. Use a proven provider or a
   maintained, well-documented library for password hashing, tokens and
   sessions.
7. Logout, password change and password reset invalidate sessions on the
   server, everywhere.
8. Denied by default: a new endpoint is protected unless it is explicitly
   made public.

## Sign-up and login

- One reusable server helper, for example `requireUser()`, runs at the top of
  every protected handler. It returns 401 when there is no valid session and
  returns the verified user. Business code never parses tokens itself.
- Tokens and sessions are verified with a library: check signature, expiry,
  issuer and audience. Reject tokens decoded without verification, the
  `alg: none` algorithm, and hard-coded or weak signing secrets.
- When an auth provider is used on the server, re-validate the user with the
  auth server on each request instead of trusting a cookie as-is.
- If auth is hand-rolled, flag it and migrate to a proven provider. Do not
  extend custom auth.
- Email verification is required before sensitive actions.
- Middleware and framework-level guards are defence in depth only. A
  middleware bypass existed in a major framework in 2025; the handler-level
  check is the one that counts. Framework "server actions" are public
  endpoints callable directly; each one carries its own auth check.
- Cap failed logins: 5 attempts per minute per IP + email combination, with
  increasing delays after repeated failures (see `SECURITY.md`, rate
  limiting). The error message stays generic.

## Passwords

- Stored with Argon2id (OWASP minimum: 19 MiB memory, 2 iterations,
  parallelism 1) or bcrypt with cost 12 (minimum 10). scrypt is acceptable.
  Anything else, including "encrypted" passwords, MD5, SHA-1, SHA-256 or
  unsalted fast hashes, is a critical defect.
- Verification uses the library's constant-time compare, never string
  equality on hashes.
- bcrypt uses only the first 72 bytes of input, so set the maximum password
  length accordingly (72 bytes).
- Password policy follows NIST guidance: minimum 8 characters, 12+
  recommended, at least 64 allowed, all characters including spaces allowed.
  No forced composition rules ("must contain a symbol"). Optionally reject
  passwords present in breach corpora via the Have I Been Pwned range API.
- Passwords and password hashes never appear in logs, error reports, emails,
  API responses (including `/api/me` and admin user lists), localStorage or
  cookies. Request loggers and error trackers are configured to redact
  password fields.
- Never email a password to anyone.
- Legacy weak hashes migrate at each user's next successful login: verify the
  old hash, then immediately re-hash with the current algorithm. Decide what
  happens to users who never log in again (for example forced reset).

## Admin and roles

- One server helper, for example `requireAdmin()`, loads the verified user
  and checks the admin role from a trusted source, returning 403 otherwise.
  It runs at the top of every admin page handler and every admin API
  handler.
- Hiding the admin link or using an unguessable URL is not protection. URLs
  end up in bundles, logs and browser history.
- List every privileged route: admin pages, admin APIs, staff dashboards,
  cron and job endpoints, and seed/test/debug routes (`/api/seed`,
  `/api/reset-db`, migration runners). Seed and test routes do not exist in
  production builds.
- A user cannot grant themselves a role by editing their own profile,
  metadata or any endpoint input.
- Every admin action is logged: who, what, when, target (see `SECURITY.md`,
  logging).
- Admin accounts use MFA where the provider supports it.

## Sessions and cookies

- Session cookies carry all three flags: `HttpOnly` (JavaScript cannot read
  them), `Secure` (HTTPS only), `SameSite=Lax` (or `Strict` for
  high-sensitivity apps when it does not break login flows). Set `Path=/`,
  the narrowest sensible `Domain`, and a bounded `Max-Age`. Use the
  `__Host-` cookie prefix where the framework allows it.
- Session ID is regenerated on login and on privilege change (prevents
  fixation). Logout destroys the session server-side, not just the cookie.
- Sessions have both an idle timeout and an absolute maximum lifetime. A
  typical consumer app: 7 to 30 days with an idle timeout. Admin sessions
  are shorter.
- A password change or reset revokes all existing sessions and emails a
  "your password was changed" notice. Provide a "log out of all devices"
  action.
- Long-lived tokens in localStorage are not used. If an auth SDK defaults to
  localStorage, either switch to its cookie mode or document why the risk is
  accepted, with a strong CSP as the mitigation.
- JWTs: short-lived access tokens with refresh-token rotation is the
  standard. The payload carries no secrets and no personal data, JWT
  contents are readable by anyone. Revocation is designed, not assumed.
- Cookies that must hold data are signed or encrypted with a strong secret.
  No secrets or personal data in readable cookies.

## OAuth, magic links and redirects

- The OAuth `state` parameter is used and verified.
- Callback and redirect URLs are an exact allowlist.
- `?next=`, `?redirect=`, `?returnTo=` parameters accept only relative paths
  on this site. No open redirects.
- Provider settings (expiry, redirect URLs) are checked in the provider
  dashboard, not only in code.

## Password reset and email links

Applies to: password reset, magic-link login, email verification, email
change confirmation, team invites, every emailed or SMS link or code that
grants access.

1. Tokens come from a cryptographically secure random source, at least
   32 bytes (`crypto.randomBytes(32)` / `secrets.token_urlsafe(32)`). Never
   timestamps, user IDs, UUIDv1 or `Math.random()`.
2. Only a SHA-256 hash of the token is stored, never the token itself.
3. Expiry: 15 to 30 minutes for password reset and magic links, up to 24
   hours for email verification, up to 7 days for invites.
4. Single use: the token is invalidated on use. Requesting a new token
   invalidates all outstanding tokens for that user. A password change
   invalidates them too.
5. The response and its timing are identical whether or not the email
   exists: "If an account exists, we've sent a link."
6. Requests are rate limited per email and per IP (`SECURITY.md`).
7. Links are built from a configured `APP_URL` environment variable, never
   from the request's Host header (host-header poisoning sends reset links
   to an attacker's domain).
8. The reset page sends `Referrer-Policy: no-referrer` and loads no
   third-party scripts. Analytics on the reset page can record the token in
   the URL.
9. After a successful reset: normal password policy and hashing apply, all
   sessions are revoked, the user is notified by email.
10. Redirect URLs used after the flow completes are allowlisted.

## Self-test (verify by attacking)

- Copy a protected API request from DevTools as cURL, delete the cookie or
  authorization line, send it: expect 401, never data.
- Open a protected page in a private window while logged out: redirect to
  login, and its data request fails.
- Change an ID in a URL or request body (`/api/items/42` to
  `/api/items/43`): never another user's data (403 or 404).
- As a normal user, request every admin page and admin API directly: 403
  everywhere, tested, not assumed.
- Check session cookies in DevTools: HttpOnly, Secure, SameSite visible.
  Replay an old cookie after logout: rejected.
- Request two password resets: the first link stops working. Reuse a used
  link: it fails. Request a reset for a fake email: identical response.
- Search the client bundle for admin emails, `isAdmin` logic and secret
  patterns: nothing found.
- Run the login attempt 6 times quickly: attempt 6 gets throttled (429) or
  delayed.

## Done when

- Every protected endpoint returns 401 without a valid session, verified by
  test, not by reading code.
- User identity always comes from the verified session; the client can name
  only its own user.
- Passwords use Argon2id or bcrypt(cost >= 12); no password material in
  logs, responses or client state.
- Roles live server-side and users cannot edit them; admin checks run in
  handlers.
- Cookies carry HttpOnly, Secure, SameSite; logout and password change kill
  sessions server-side.
- Reset and magic links are random, hashed, single-use, expiring, rate
  limited and host-independent.

Basis: Pre-Launch Security Toolkit, checks 01, 05, 09, 15, 17.
