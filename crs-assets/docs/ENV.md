# ENV.md

Environment variable and secret rules for this project. Any AI coding agent
working on this repository follows this file.

## Hard rules (never break)

1. Anything the browser can see, every visitor can see. Secret keys live on
   the server only. Minified code is not hiding: bundled JavaScript is
   trivially searchable.
2. Never hard-code a key in source, config, scripts, tests, documentation,
   screenshots or chat. If a key is written in code, it is already leaked.
3. Public env prefixes (`NEXT_PUBLIC_`, `VITE_`, `REACT_APP_`,
   `EXPO_PUBLIC_`, `PUBLIC_`, `NUXT_PUBLIC_`) ship the value into the
   browser bundle. A secret never uses a public prefix and is never read
   from client-side code, no matter what the prefix is.
4. `.env`, `.env.local`, `.env.*.local` are gitignored. Only
   `.env.example` (names + safe placeholders, never real values) is
   committed.
5. No insecure fallbacks. `process.env.JWT_SECRET || "secret"` and
   `?? "changeme"` are defects. The app fails fast at startup with a clear
   message when a required variable is missing, without printing its value.
6. Development and production use different values: test keys in dev, live
   keys only in production, separate dev database.
7. Signing secrets are long random values, at least 32 bytes, from a
   secure generator. Never a phrase, never reused across projects.
8. Never paste real secret values into an AI chat, an issue, a support
   ticket or a screenshot. Refer to keys by variable name.

## Key classification

| Class | Examples | Where it lives |
|---|---|---|
| Usually publishable | Stripe `pk_` keys, Supabase anon/publishable key (only with RLS on), Firebase web config, domain-restricted browser map keys | Client |
| Always secret | Stripe `sk_`/`rk_` keys, OpenAI/Anthropic/AI keys, Supabase `service_role` key, database URLs, email/SMS provider keys, webhook signing secrets, JWT/session secrets, cloud credentials | Server only |

Publishable keys stay public only with dashboard restrictions applied:
allowed domains or referrers, allowed APIs, read-only scopes.

Every client call that needs a secret goes through a small server route
instead: browser calls this server, the server adds the key. That route
requires authentication where appropriate, validates input, is rate
limited, and performs only the one operation the UI needs. A generic
proxy forwarding any URL or body is an open relay and is not built.

## Inventory

- Every env var is documented in `.env.example`: name, safe placeholder,
  and a short comment stating where it is used (client or server) and what
  breaks without it.
- A `SECRETS.md` (or wiki page) lists every credential: name, purpose,
  owner, where stored, last rotated. Values never appear in it.
- Secrets are stored in the hosting platform's environment settings per
  environment, or in a secret manager (Infisical free tier works for small
  teams). Local `.env` files are for development only.
- Changing an env var on most hosts requires a redeploy to take effect;
  remember that when rotating.

## Leak response

A key that was ever in client code, Git history, a public file, logs, a
screenshot or a shared chat is treated as stolen, whether or not it still
appears in the code: create the replacement, deploy it, then revoke the
old one and confirm it returns 401. Full procedure in `SECURITY.md`
(key rotation). Adding `.env` to `.gitignore` does not remove it from Git
history; that needs a history scan.

## Self-test

- `git ls-files | grep -i env`: only `.env.example` appears.
- Production build, then search the output (`dist/`, `build/`) for
  `sk_`, `sk-`, `secret`, `Bearer`, `api_key`, `service_role`: nothing
  found.
- Live site, DevTools: Sources search for the same words; Network tab
  request URLs and headers contain no keys.
- No secret variable name carries a public prefix.
- Search the codebase for fallback patterns (`|| "secret"`,
  `?? "changeme"`): none.
- `/api/config` or `/api/health` endpoints expose no values, only names or
  status.
- Run GitGuardian or gitleaks over the repo: zero unresolved findings.

## Done when

- No secret in source, client bundle, build output or Git history.
- No secret behind a public prefix or read by client code.
- `.env.example` documents every variable; `.env` gitignored; startup
  validation fails fast.
- Dev and production keys and databases are separate.
- Secret inventory exists without values; leaked keys, if any, rotated.

Basis: Pre-Launch Security Toolkit, checks 02 and 11; App-Performance
Toolkit project-context rules (never paste secrets into prompts).
