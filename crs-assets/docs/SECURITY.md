# SECURITY.md

Security rules for this project. Any AI coding agent working on this
repository follows this file. Human developers follow it too.

This is the master file. Deep dives live in sibling files:

- Identity, sessions, passwords, admin, resets: `AUTH.md`
- Database row access and RLS policies: `RLS-POLICIES.md`
- Environment variables and secrets: `ENV.md`
- Threat actors and assets overview: `THREAT-MODEL.md`

## Hard rules (never break)

1. The server is the source of truth for identity, roles, prices, totals and
   entitlements. The browser is user-controlled territory; anything sent
   from it is input, never authority.
2. Every input is validated on the server. Front-end validation is UX only.
3. User text is rendered as text, never as code.
4. Database queries are parameterized. User input is never concatenated into
   query text, shell commands or ORM raw calls.
5. Secrets never live in code, client bundles, logs or screenshots. They live
   in server-side environment variables (`ENV.md`).
6. Public routes are rate limited.
7. Security headers are on every route, including API responses.
8. Production never runs debug mode, never returns stack traces, SQL errors
   or internal messages.
9. No new dependency is installed without checking it is real, maintained
   and legitimate. AI tools invent package names; attackers register them.
10. Banned in this project: PHP, Next.js, Electron. Do not propose or
    generate code for them. For desktop apps use the project's actual
    approach (Rust shell, e.g. Tauri), never Electron.

## Red flags (drop everything if any is true)

- A secret key anywhere in front-end code or the client bundle.
- A database table reachable from the browser with no row-level policy.
- Changing an ID in a URL shows another user's data.
- An admin page "protected" only by hiding the link.
- A price or total trusted from the browser.
- A plan or order unlocked on a "success" page instead of a verified webhook.
- A `.env` file ever committed, or `https://<domain>/.env` returning
  anything but 404.
- An AI endpoint with no login requirement or usage limit.
- Stack traces or SQL errors on the live site.
- Debug/test/seed routes reachable in production.

## Input validation (server-side)

- One validation library for the stack (schema-based, e.g. Zod in
  TypeScript, Pydantic in Python). Validation runs at the start of every
  handler, before any business logic or database call.
- Every field gets: a type; required or optional; min/max for numbers; a
  maximum length for every string; allowed values (enums) where possible;
  format checks (email, URL, UUID, date). Money is an integer in minor
  units (cents), never floating point.
- Unknown fields are rejected (strict mode). Client input is copied field by
  field into database writes; `...body` spreads into an ORM call are a
  critical defect.
- Server-controlled fields never come from client input: `id`,
  `user_id`/`owner_id`, `role`, `is_admin`, `price`, `amount`, `status`,
  `credits`, `balance`, `plan`, `is_verified`, `email_verified`,
  `created_at`.
- Inputs are normalized before saving (trim whitespace, lowercase emails).
- Invalid input returns a clean 400 with field-level messages, never a
  crash, stack trace or database error.
- A maximum request body size is enforced.
- Runtime types are not validation: a TypeScript `number` field still
  receives `"quantity": "-3"` over the wire.

## XSS

- Framework escaping is the default. Audit every escape hatch with the data
  origin: `dangerouslySetInnerHTML`, `v-html`, `{@html}`, `bypassSecurityTrust*`,
  `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, jQuery
  `.html()/.append()`, server-template `|safe`/`mark_safe`/`autoescape off`,
  `eval`, `new Function`, string-based `setTimeout`.
- Where HTML is genuinely needed, sanitize with a maintained sanitizer
  (DOMPurify or equivalent) using a strict tag and attribute allowlist.
  Sanitize on output, not only on input: data enters the database from
  imports, APIs and old versions too.
- User-controlled URLs in `href`, `src`, `action`, `window.location` allow
  only `http:`, `https:`, `mailto:`. `javascript:` and `data:` are blocked.
- Markdown and rich-text rendering allowlists raw HTML (usually: no raw
  HTML at all).
- AI/LLM output is untrusted user input. It is sanitized before rendering
  and validated before use in queries, URLs or commands.
- Admin panels are a prime target: stored XSS firing when an admin views
  user content takes over the app.
- Self-test: submit `<img src=x onerror=alert(1)>` and `javascript:alert(1)`
  through every displayed field. Any alert or executable link anywhere is a
  defect.

## Injection

- All queries use parameterized or bound parameters or a safe ORM query
  builder. Flag any query text built with concatenation, template literals,
  f-strings, `.format()` or `%`.
- Known ORM escape hatches to treat as dangerous with user input:
  `$queryRawUnsafe` / `$executeRawUnsafe` (Prisma; the tagged-template
  `` $queryRaw`...${x}` `` form IS parameterized and safe),
  `sequelize.literal()`, `knex.raw()` with interpolation, TypeORM `query()`
  with strings, SQLAlchemy `text()` built with f-strings, Django
  `raw()`/`extra()`, Drizzle `sql.raw()`.
- Dynamic identifiers that cannot be parameterized (ORDER BY column, sort
  direction, table or column names, LIMIT) map input to a hard-coded
  allowlist. The raw value is never inserted.
- NoSQL: validate input types before queries; request JSON passed straight
  into a query enables operator injection (`{"$ne": null}`).
- PostgREST-style filter strings built from user input
  (`.or(\`name.eq.${input}\`)`) and RPC functions building dynamic SQL are
  flagged.
- Command injection: no user input reaches `exec`, `spawn` with
  `shell: true`, `os.system` or `subprocess` with `shell=True`. Use argument
  arrays without a shell.
- Self-test: send `' OR '1'='1`, `'; DROP TABLE users; --` and
  `{"$ne": null}` to every affected input. Normal empty results or 400 is
  correct; extra data or a database error is a defect.

## CSRF

- If login uses cookies, CSRF applies. Every state-changing request (POST,
  PUT, PATCH, DELETE, server action, form handler) requires a CSRF token or
  passes an Origin check: reject when Origin is present and is not this
  site.
- No GET route changes state. `/delete?id=`, `/logout` as GET, `/subscribe`
  as GET: all move to POST/DELETE.
- JSON endpoints require `Content-Type: application/json` (form-encoded and
  text/plain bodies can be sent cross-site without a preflight).
- Bearer-token-only APIs (Authorization header added by JavaScript, no
  cookies) have low CSRF risk; confirm no cookie-based path exists.
- High-risk actions (change email, change password, delete account, payouts)
  additionally require the current password or a recent re-login.
- Webhooks are exempt from CSRF but must verify their signature (payments
  section).
- Login forms are protected too: login CSRF (logging the victim into the
  attacker's account) is real.

## Rate limiting

- In-memory counters do not work on serverless or multi-instance hosting.
  Use a shared store (Redis-based limiter, database, or the platform's rate
  limiting feature).
- Key by user ID when signed in, by IP otherwise. Get the client IP from the
  platform's verified header; a raw `X-Forwarded-For` is spoofable.
- Default limits (adjust per project, but never weaker without a written
  reason):

  | Endpoint group | Default limit |
  |---|---|
  | Login | 5 attempts/min per IP + email, increasing delays |
  | Sign-up | 5/hour per IP |
  | Password reset / magic link | 3/hour per email and 10/hour per IP |
  | OTP / MFA codes | 5 attempts per code, then invalidate the code |
  | AI/LLM endpoints | per-user per-minute limit plus a daily quota per user/plan |
  | Email/SMS sending | strict per-user and global caps; SMS restricted by destination country |
  | General API | generous per-IP cap (e.g. 100/min) |

- Over-limit requests get 429 with a `Retry-After` header and a friendly
  message that does not reveal whether an account exists.
- Rate-limit hits are logged (`SECURITY.md`, logging).
- Paid and metered APIs (AI, email, SMS) additionally have hard spending
  caps set in each provider's dashboard as the last line of defence.

## Security headers and HTTPS

Applied to all routes, in the one right place for the stack:

- `Content-Security-Policy`, starting from:
  `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'; upgrade-insecure-requests`.
  Add only third-party origins the app actually loads or connects to. Avoid
  `'unsafe-inline'` and `'unsafe-eval'` for scripts; if inline scripts are
  required, use nonces or hashes and say why.
- `Strict-Transport-Security: max-age=63072000; includeSubDomains` (add
  `includeSubDomains` only when every subdomain serves HTTPS; skip
  `preload` until the domain is stable and explicitly confirmed).
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY` (or `SAMEORIGIN` only if the app frames its own
  pages)
- `Referrer-Policy: strict-origin-when-cross-origin`
- `Permissions-Policy: camera=(), microphone=(), geolocation=()`, enabling
  only features the app uses
- `Cross-Origin-Opener-Policy: same-origin` (`same-origin-allow-popups` if
  OAuth or payment popups break)
- Remove `X-Powered-By` and server version headers.
- `Cache-Control: no-store` on authenticated pages and API responses with
  personal data.
- HTTPS everywhere: every HTTP request on every domain, subdomain and path
  permanently redirects (301/308) to HTTPS. No hard-coded `http://` in API
  calls, images, scripts, fonts, iframes, webhooks, OAuth callbacks, email
  links or redirects. Database and service connections use TLS (Postgres
  connection strings include `sslmode=require`). Cookies carry `Secure`.
  Behind a proxy, the original protocol is read correctly (`trust proxy`)
  so redirects do not loop.
- CSP rollout: ship as `Content-Security-Policy-Report-Only` first, walk
  every main page (login, checkout, OAuth, payments, uploads, embeds),
  adjust, then enforce.
- Verify: securityheaders.com grade A or better; SSL Labs grade A; no mixed
  content warnings in the console.

## CORS

- CORS is not authentication. It only controls browsers; curl and servers
  ignore it. Endpoints enforce auth regardless.
- No `Access-Control-Allow-Origin: *` on endpoints using cookies or
  returning private data. No reflecting the request origin without checking
  it. No wildcard combined with `Allow-Credentials: true`. No "null"
  origin. No loose matching (`endsWith("example.com")` matches
  evilexample.com).
- Allowed origins are an exact allowlist from environment config:
  production domains, localhost only in development.
- Methods and headers restricted to what the front end uses.
- A public, read-only, non-personalized endpoint may use `*` but never with
  cookies.
- Cloud storage buckets have their own CORS settings; check them separately.
- Verify: `curl -s -I -H "Origin: https://evil.example" https://<domain>/api/ANY`
  returns no `Access-Control-Allow-Origin` header.

## File uploads

All checks enforced on the server or in storage policies, never only in the
browser.

- Per feature, an allowlist of types, verified by real content (magic
  bytes), never by client Content-Type or extension alone. HTML, SVG, XML,
  JS and executables are blocked unless a feature truly needs them.
- Size limits per file enforced server-side and in presigned upload
  conditions (`content-length-range`) or bucket limits. Limits on files per
  request and a per-user storage quota.
- Storage names are generated (UUIDs). The user's filename never becomes a
  storage path (path traversal); it is kept only as escaped display text.
- Storage is object storage or a folder outside the web root, private by
  default. Private files are served through short-lived signed URLs after an
  authorization check. Public buckets are not listable.
- Serving: correct Content-Type, `X-Content-Type-Options: nosniff`,
  `Content-Disposition: attachment` for anything that is not a displayed
  image. Serving user files from a separate domain is preferred.
- Images are re-encoded (sharp, Pillow or equivalent), which strips hidden
  payloads and EXIF data including GPS location. Pixel dimensions are
  capped (decompression bombs).
- Archives: cap uncompressed size and file count. CSV exports: prefix cells
  starting with `=`, `+`, `-` or `@` to prevent spreadsheet formula
  injection.
- User-to-user shared files are scanned for malware.
- Presigned uploads skip the server; limits must live in the presigned
  policy or bucket rules.
- Self-test: rename `.html` to `.jpg` and upload; upload an SVG containing a
  script; upload an oversized file; upload a `/../` filename. All rejected
  or neutralized; uploaded files never execute as pages on the main domain.

## Payments

Even when no payments exist yet, this section governs any future billing,
credits or balances in this project.

- The server computes every price, tax, shipping, discount and total from
  its own database or provider price IDs. Any price, amount, currency or
  total sent by the client is ignored.
- The client sends only product or plan IDs and quantities. Quantities are
  integers, minimum 1, with a sensible maximum.
- Money is an integer in minor units (cents). No floating point.
- Coupons and credits are validated server-side (exists, not expired, usage
  limits, per-user limits, eligible products) and applied inside a database
  transaction so parallel requests cannot spend one twice.
- Fulfilment happens only from a signature-verified webhook
  (`checkout.session.completed`, `payment_intent.succeeded`, `invoice.paid`
  or the provider's equivalent). Landing on a success page never unlocks
  anything; the success page only shows status.
- Webhook signatures are verified with the raw request body and the
  endpoint secret before parsing. Many frameworks need raw-body config on
  that route; parsing JSON before verifying breaks the check.
- Processed webhook event IDs are stored (idempotency) so duplicates are
  ignored. Out-of-order events are handled. The amount and currency paid
  are checked against the order.
- Subscriptions and entitlements are decided on the server from
  subscription status stored via webhooks. Never from a client flag or a
  query parameter like `?plan=pro`. Cancellations, failed payments, refunds
  and disputes remove access.
- Balances, credits and stock update atomically in transactions (race
  conditions double-spend).
- Card data never touches this server: provider-hosted checkout or elements
  only.
- Test keys in development, live keys only in production.
- Retries use the provider's idempotency keys. Never blindly retry a payment
  request after a timeout.
- Free trials and referral logic cannot be reused endlessly with new email
  addresses; they are business-logic money leaks.
- Self-test: edit the price in the checkout request (charged amount must not
  change); visit the success URL without paying (nothing unlocked); reuse a
  coupon; send an unsigned and a duplicate webhook. All fail safely. Use the
  provider CLI to send test webhooks.

## Errors and debug mode

- Production error responses show a generic message plus a request ID. Full
  details go to server logs and error tracking only. Returning
  `error.message` from a catch block is a leak: database errors include
  table and column names.
- Debug mode is off via environment config so it cannot be forgotten
  (`NODE_ENV=production`, `APP_DEBUG=false`, `DEBUG=False`, `debug=False`
  per framework). A Flask/Werkzeug-style debugger is remote code execution.
- API docs, GraphQL introspection and playgrounds are disabled or protected
  in production unless intentionally public.
- Source maps are not served publicly (upload privately to the error
  tracker instead). Directory listing is off. Noisy `console.log` calls and
  debug-only routes are removed from production builds.
- Health endpoints do not reveal versions, environment or dependency
  details.
- Preview and staging deployments are protected with a password or platform
  auth; they are public by default and often run debug mode.
- Self-test: trigger a 404, a 500 and a validation error on production and
  check that no stack trace, path, query or version appears. Check
  `/docs`, `/swagger`, `/graphql` return 404 or auth.

## Dependencies

- Audit before launch and then regularly: `npm audit --omit=dev` (or the
  package manager's equivalent), `pip-audit`, `cargo audit`, `osv-scanner`.
  Enable Dependabot or Renovate with grouped updates and GitHub security
  alerts.
- The core framework and runtime stay on supported, patched versions.
- Upgrades: production Critical/High first, smallest version bump containing
  the fix. Major upgrades: read the changelog, list breaking changes,
  fix them. Vulnerable transitive deps: upgrade the parent or use overrides,
  explaining why. Never `npm audit fix --force` blindly. Build and test
  after each group of upgrades.
- Every direct dependency is verified to be real: exists on the official
  registry, is the well-known package (not a typo-squat), has a real
  repository and plausible download stats. Flag anything an AI may have
  invented (slopsquatting) and packages with unexpected install scripts.
- Lockfile is committed; CI installs from it (`npm ci`,
  `--frozen-lockfile`).
- Unused dependencies are removed and dev-only tools live in
  devDependencies. Dead code is removed too: unused routes, pages for
  dropped features, old webhook endpoints, test and seed routes, unused
  third-party scripts, and OAuth apps or API keys tied to removed features
  (revoked in their dashboards). Build and test after cleanup.

## Exposed files

- Nothing private is downloadable: `.env` files, `.git`, database dumps,
  archives, logs, IDE folders, `.DS_Store`, internal configs, fixtures with
  real data.
- Public folders (`public/`, `static/`, build output) contain only public
  files. Dotfiles and backup/dump extensions return 404 via host config or
  middleware. Directory listing is off.
- `robots.txt` is public and is not protection; it must not advertise
  private paths either.
- Storage buckets are private unless they must be public, and never
  listable.
- `.dockerignore` / deploy-ignore files keep secrets out of images and
  deployments.
- Keep a `check-exposed.sh` script that probes the live domain for:
  `/.env`, `/.env.local`, `/.env.production`, `/.env.backup`, `/.git/config`,
  `/.git/HEAD`, `/.DS_Store`, `/backup.zip`, `/db.sql`, `/dump.sql`,
  `/database.sqlite`, `/config.json`, `/docker-compose.yml`, `/openapi.json`,
  `/graphql` and stray `.map` files. Every line should report 404 or 403.
  Re-run after every major deploy.
- If `.env` was ever publicly reachable or committed, every key in it is
  treated as compromised and rotated (`ENV.md`, leak response).

## Git history and leaked secrets

- Deleting a secret in a new commit does not remove it. Scan the full
  history, all branches and tags, with gitleaks or TruffleHog. Manual
  fallback: `git log --all -p -S "<pattern>"` and
  `git log --all --full-history -- "**/.env*"` for patterns:
  `sk_live_`, `sk-`, `sk-ant-`, `AKIA`, `ghp_`, `github_pat_`, `xoxb-`,
  `AIza`, `-----BEGIN PRIVATE KEY-----`, `"service_role"`,
  `postgres://...@`, `api_key=`, `secret=`, `password=`, long random
  strings in config files, and every `.env` ever committed.
- Every secret found goes on the rotation list. Rotating comes first;
  rewriting history does not make a leaked key safe (forks, clones and
  caches keep copies).
- Prevention: pre-commit secret scanning hook, a CI scan on every push,
  GitHub Secret scanning and Push protection enabled.
- History purge (`git filter-repo`) only after rotation and only when
  wanted: it rewrites commit hashes, force-pushes, and every collaborator
  re-clones.
- Never print full secret values anywhere; mask to the first 4 characters.
- Verify: run GitGuardian (free for individuals and small teams) or gitleaks
  over the repo; zero unresolved findings.

## Key rotation

Any key ever exposed, anywhere, is treated as stolen: replaced, deployed,
then revoked.

1. Inventory every credential: API keys (AI, payments, email, SMS, maps),
   database passwords and connection strings, service-role keys, JWT and
   session signing secrets, OAuth client secrets, webhook signing secrets,
   cloud access keys, deploy and CI tokens, SMTP credentials. Record where
   each is stored and the evidence it was exposed.
2. Prioritize by blast radius: payments and cloud infrastructure first, then
   database and service-role keys, then AI and messaging (cost abuse), then
   the rest.
3. Per key: create the replacement with minimum scopes and restrictions
   (allowed domains/IPs, spending limits); update every place it is used
   (hosting env vars per environment, CI, local `.env`); redeploy and
   confirm the app works; revoke the old key; confirm the old key now fails
   (401/403) with a safe read-only call; check provider usage logs for
   misuse during the exposure window.
4. Side effects to plan: rotating a JWT/session secret logs everyone out;
   rotating a database password means updating every connecting service;
   rotating a webhook secret means updating the provider endpoint config.
5. Keep a `SECRETS.md` inventory: names, purpose, owner, where stored, last
   rotated. Never values.
6. Never deploy a rotation by revoking the old key first; that is a
   self-inflicted outage.

## Logging and monitoring

- One helper, e.g. `logSecurityEvent(event, details)`, emits structured JSON
  events: login success/failure, logout, sign-up, password change, reset
  requested/completed, email change, MFA changes, permission denied
  (401/403), admin actions and role changes, rate-limit hits (429), payment
  and webhook events including signature failures, file uploads, API key
  creation, unexpected 5xx.
- Every event carries: UTC timestamp, event name, user ID if known, IP
  address, user agent, request ID, outcome. The request ID appears in error
  responses so a user report maps to a log line.
- Redaction everywhere: passwords, tokens, cookies, Authorization headers,
  API keys, reset links, full card numbers. The error tracker scrubs too.
  User-supplied values are stripped of newlines before logging (log
  injection).
- Alerts: more than 10 failed logins for one account or IP in 5 minutes;
  spikes in 401/403/429/5xx; any new admin or role change; webhook
  signature failures.
- Logs go somewhere with retention (serverless platform logs can vanish
  fast). Log access is limited to the team. Retention period is defined.
  Log IDs, not full profiles: logging too much personal data is a privacy
  problem.
- Keep a short runbook: where logs live, how to search one user's activity,
  what each alert means, the first three actions when one fires.

## AI features

- Assume any user text, file, email or web page fed to the model contains
  hostile instructions (prompt injection). The model cannot reveal secrets:
  no keys and no other users' data go into prompts.
- The model cannot access data or trigger tools beyond the current user's
  permissions. Destructive or costly actions need explicit user
  confirmation.
- Model output is untrusted: sanitize before rendering as HTML/Markdown,
  validate before using in queries, URLs or commands.
- AI endpoints require authentication, per-user rate limits and daily
  quotas, maximum input and output tokens, and a provider-side monthly
  spending cap.
- Send the minimum personal data to providers. Log prompts carefully or not
  at all. Check vendor terms for training on your data (`LEGAL.md`).

## SSRF (server-side request forgery)

Any feature fetching a user-influenceable URL (link previews, import from
URL, image from URL, outgoing webhooks, PDF or screenshot generation, AI
browsing):

- Allow only `http:` and `https:`.
- Resolve the hostname and block private, loopback, link-local and internal
  ranges for the initial request and every redirect: `127.0.0.0/8`,
  `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`
  (including the cloud metadata address 169.254.169.254), `::1`,
  `fc00::/7`.
- Prefer a domain allowlist where possible.
- Short timeouts and a maximum response size.
- Never return raw internal error details.
- Test with internal addresses.

## Backups

- Database, file storage and configuration are backed up on a schedule with
  a defined retention. Point-in-time recovery is used when the plan offers
  it.
- Backups are protected from the same credentials as production: a mistake
  or attacker cannot delete both.
- A `RESTORE.md` exists: step-by-step restore to a new environment, plus how
  to run a test restore without touching production. A restore is actually
  tested periodically; an untested backup is a hope, not a backup.

## Your own accounts

The easiest way in is through the maintainer's accounts.

- MFA everywhere: code hosting, hosting/deploy, DNS and domain registrar,
  database dashboard, payments, email, AI providers, analytics, error
  tracking. Authenticator app or passkey, not SMS where avoidable.
- Least-privilege roles; unneeded team members and tokens removed; connected
  OAuth apps and integrations reviewed; security alerts on; domain lock on
  at the registrar.

## Spending caps and abuse alarms

- Every usage-billed service (hosting, serverless, database, storage,
  bandwidth, AI APIs, email, SMS, maps, search) has: a billing alert, a hard
  spending cap or quota where supported, per-key usage limits.
- App-level per-user quotas on the most expensive features; a daily usage
  summary in the logs.
- SMS destinations are restricted by country (SMS pumping fraud).

## Email domain

- SPF (one record listing all senders), DKIM (from each provider), DMARC
  (start at `p=none` with a reporting address, move to quarantine/reject
  once reports are clean).
- App emails include no sensitive data beyond need; links use the
  configured `APP_URL`.
- Verify the setup with a free SPF/DKIM/DMARC checker after every DNS
  change.

## Full review and security tests

- After the checklists, run one full pass as an attacker would, covering:
  auth, access control (IDOR on every ID-based endpoint, tenant and team
  isolation), secrets, data, inputs, files, payments, network, packages and
  deployment, business logic (trial abuse, invite abuse, skipped steps,
  race conditions), and AI features. Business-logic flows unique to the
  project are where checklists miss things.
- Write findings as a report: severity, location (file:line or endpoint),
  how an attacker would exploit it step by step, fix, effort. Fix Critical
  and High first. Ask "show me the exact code that proves this" for any
  AI-generated finding that looks off; AI reviewers miss real problems and
  invent fake ones.
- Prove every protection with an automated "try to break it" test:
  signed-out 401s, cross-user 403/404, tampered prices, expired tokens,
  oversize uploads, rate-limit 429s, foreign-Origin CSRF rejections,
  unsigned webhooks. Tests run in CI on every push. Never weaken or delete
  a test to make it pass.
- Tests run against local or staging only. Never destructive tests against
  production or real user data.
- AI reviewers raise the baseline; they are not a guarantee. Anything
  touching payments, health data, children's data or large user counts gets
  a professional review before launch.

## Re-run matrix

After adding a feature, re-check the affected areas:

| When you add... | Re-read / re-check |
|---|---|
| Login or sign-up | `AUTH.md` + rate limiting + CSRF |
| A database table | `RLS-POLICIES.md` + input validation + injection |
| An API route or server function | auth check + validation + rate limit + logging |
| Comments, profiles, any user text | XSS + rate limiting |
| Payments or subscriptions | payments section + rate limiting + logging |
| File uploads | uploads section + RLS + rate limiting |
| A third-party API or key | `ENV.md` + spending caps |
| An admin tool | `AUTH.md` admin + logging + XSS |
| AI/LLM features | AI section + quotas |
| Fetching user-supplied URLs | SSRF section |
| A package | dependencies section (verify it is real first) |
| Before every launch | exposed files + debug mode + full review + security tests |

## Emergency card

Triggers: leaked key, strange charges, users reporting actions they did not
take, data changed on its own.

1. Leaked payment, cloud or AI key: revoke it in the provider dashboard
   immediately, even before deploying a replacement. Minutes of downtime
   are cheaper than the alternative. Then rotate properly (key rotation
   section).
2. Keep evidence: do not delete logs, users or the repo. Write down what
   was noticed and when.
3. Stop the bleeding: rotate exposed keys (payments/cloud/database first),
   maintenance mode on the affected feature if needed.
4. Kill sessions: rotate session/JWT secrets or revoke sessions so attackers
   are logged out; force password resets for affected accounts.
5. Investigate: app logs, database logs, provider dashboards. When did it
   start, what was accessed, which accounts.
6. Close the hole: fix the vulnerability, add a test that proves it, deploy.
7. Check the data: was anything read, changed or deleted? Are backups
   intact?
8. Tell people: if personal data was affected, notify users honestly.
   GDPR may require regulator notification within 72 hours; get legal
   advice (`LEGAL.md`).
9. Learn: short post-mortem: cause, impact, fix, what prevents recurrence.

## Post-launch rhythm

- Every commit: secret scan (pre-commit hook).
- Every push/deploy: security tests in CI; exposed-file probe after big
  deploys.
- Weekly: review Dependabot/Renovate PRs and security alerts; glance at
  failed-login and 5xx alerts.
- Monthly: dependency audit, unused-package cleanup, admin user and team
  access review, billing dashboard check.
- Quarterly: full security review, security test run, backup restore test,
  rotate high-value keys.
- Every big feature: run the re-run matrix before shipping.

## Verify tools

securityheaders.com (header grade), SSL Labs (TLS grade), gitleaks or
TruffleHog and GitGuardian (secret scanning), HostedScan (vulnerability
scans, free tier), Infisical (secret storage), osv-scanner / npm audit /
pip-audit / cargo audit (dependencies).

Basis: Pre-Launch Security Toolkit, checks 02, 03, 04, 06, 07, 08, 10-14,
16, 18-27, bonus B1-B7, emergency card and post-launch guidance.
