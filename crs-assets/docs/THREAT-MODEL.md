# THREAT-MODEL.md

What this project protects, from whom. Any AI coding agent working on this
repository reads this file before designing or changing features, and
extends it when the project grows.

## The base model

### Assets (protect these)

| Asset | Why |
|---|---|
| User data | Accounts, personal content, private rows; leaks destroy trust and may trigger legal duties (`LEGAL.md`) |
| Payments and money | Prices, balances, credits, subscription state; abuse is direct loss |
| Keys and credentials | API keys, session tokens, signing secrets; one leak can cost unlimited money or data |
| Availability and budget | Uptime and metered-service spend; a bot loop can run up a large invoice overnight |
| Reputation and honest claims | What the product promises in UI, emails and listings |

### Threat actors (protect from these)

| Actor | Typical attack |
|---|---|
| Bots and automation | Credential stuffing, sign-up spam, scraping paid endpoints, AI-quota abuse, SMS pumping |
| Other users | IDOR (changing an ID to reach someone else's rows), privilege escalation, tenant isolation breaks, XSS against admins |
| Scrapers | Bulk extraction of public and semi-public data |
| Supply chain | Vulnerable or fake dependencies, leaked keys in Git history, compromised vendors |
| Compromised or phished maintainer accounts | Hosting, domain, database, payment dashboards; bypasses every app-level check |
| Ourselves by mistake | Bad migrations, debug mode left on, secrets committed, backups missing |

## STRIDE mapping

| Threat | Where it lands in this repo |
|---|---|
| Spoofing (pretending to be a user) | `AUTH.md`: server-side auth on every request, generic errors, session flags |
| Tampering (changing data they should not) | `SECURITY.md`: server-side validation, parameterized queries, mass-assignment blocks |
| Repudiation (denying actions) | `SECURITY.md`: security event logging with user, IP, request ID |
| Information disclosure | `RLS-POLICIES.md`: row-level policies; cookies, headers, CORS, no debug output (`SECURITY.md`) |
| Denial of service and cost abuse | `SECURITY.md`: rate limiting, quotas, spending caps |
| Elevation of privilege | `AUTH.md`: server-side role checks, trusted role sources; `RLS-POLICIES.md`: WITH CHECK |

## Trust boundaries (standing assumptions)

- The browser is untrusted. Everything it sends is input, never authority.
- All user input is untrusted, including text, files, URLs and metadata.
- AI model output is untrusted user input.
- Third-party API responses are untrusted input to our validation.
- "Nobody knows the URL" is not access control.
- A private repo is not secret storage; history keeps everything.
- Admins are users: admin inputs get the same validation and logging.
- Localhost and staging are not secure environments.
- Preview deployments are public until protected.

## The recurring attacks for AI-built apps

Ranked by how often they actually happen: secret keys in front-end code or
bundles; missing row-level policies on a browser-reachable database; IDOR
by changing IDs; admin pages hidden only in the UI; prices trusted from the
client; orders unlocked on a success page instead of a verified webhook;
committed `.env` files; AI endpoints without auth or quotas; stack traces
on the live site; vulnerable or hallucinated dependencies. Every feature
review starts by ruling these out.

## Per-entry-point checklist

Every new page, API route, server function, webhook, cron job, upload path
and AI endpoint answers six questions before it ships:

1. Does it require a signed-in user, and is that checked on the server
   inside the handler?
2. Which roles are allowed, and where is the role stored?
3. Is every input validated server-side (type, length, range, format,
   allowed fields)?
4. Is it rate limited, and does it carry a spending cap if it costs money?
5. Is the action logged with user, IP and request ID?
6. What can it touch: which rows (whose?), files, keys, spend?

## Extending this model per project

The tables above are the universal base. When this project adds a feature
class, add a row to the asset table and walk the actors against it:

- Payments or credits appear: add "double-spend via race conditions",
  "coupon reuse", "success-page unlock" to the actor rows for other users.
- Group, family or team features appear: add "membership boundary break"
  (a member of group A reading group B's rows) and require the shared-group
  policy pattern from `RLS-POLICIES.md`.
- File uploads appear: add "malicious file served on our domain" and follow
  the uploads section in `SECURITY.md`.
- AI features appear: add "prompt injection", "quota abuse" and follow the
  AI section in `SECURITY.md`.
- URL fetching appears: add "SSRF into internal network" and follow the
  SSRF section.
- Children become a target audience: add COPPA duties and follow
  `LEGAL.md` before any data collection.

Update triggers: any new feature class, any new third-party service, any
change to who the users are, and always before a launch. Review the whole
file at each quarterly security review (`SECURITY.md`, post-launch
rhythm).

Basis: Pre-Launch Security Toolkit (Priority Eight, red flags, full review
areas); STRIDE; the docs-model video (protect user data, payments, keys
from bots, other users, scrapers).
