# DOCS-INDEX.md

Standing rules for this repository. Any AI coding agent working here reads
this file first, then the file relevant to the task. These files are
instructions, not suggestions.

Last updated: 2026-11-12.

## The files

| File | Governs |
|---|---|
| `AUTH.md` | Sign-up, login, sessions, passwords, admin roles, OAuth, password reset |
| `SECURITY.md` | Master security file: validation, XSS, injection, CSRF, rate limits, headers, CORS, uploads, payments, dependencies, logging, AI, SSRF, backups, incident response |
| `RLS-POLICIES.md` | Row-level security: policy patterns, decision table, storage rules |
| `ENV.md` | Environment variables, secret classification, leak response |
| `THREAT-MODEL.md` | Assets, threat actors, STRIDE mapping, per-entry-point checklist |
| `LEGAL.md` | Privacy, consent, data rights, children, accessibility, fair pricing, honest claims, IP, per-unit legal risks |
| `PERFORMANCE.md` | Speed budgets, measurement method, optimization rules |
| `SEO.md` | Titles, meta descriptions, Open Graph, sitemap, Search Console |
| `GLOSSARY.md` | One-line definitions for terms used across these files |

## When to read what

Before any implementation work: read `THREAT-MODEL.md` once to know what
this project protects. Then, when the task touches:

- login, sign-up, sessions, passwords, roles: `AUTH.md`
- any input, query, header, upload, payment, dependency, log: `SECURITY.md`
- a database table or any data access: `RLS-POLICIES.md`
- a key, env var or third-party service: `ENV.md`
- a new feature type or entry point: `THREAT-MODEL.md` (extend it)
- anything user-facing that collects data, prices, or ships to the EU/US:
  `LEGAL.md`
- any page, component or network call: `PERFORMANCE.md`
- any public page, route metadata or share preview: `SEO.md`
- an unfamiliar term: `GLOSSARY.md`

Cross-cutting tasks read several files. A payments feature touches
`SECURITY.md` (payments, rate limits, logging), `AUTH.md` (who can buy),
`LEGAL.md` (renewal disclosure, refunds) and `PERFORMANCE.md` (API
budgets) at once.

## Rules that span every file

1. Banned technologies in this project: PHP, Next.js, Electron. Never
   propose, generate or migrate toward them. Desktop apps use the
   project's actual approach (Rust shell, e.g. Tauri).
2. The server is the source of truth for identity, roles, prices and
   entitlements. The browser proves nothing.
3. Secrets never enter code, client bundles, logs, screenshots or AI
   chats; refer to keys by variable name only.
4. A new endpoint, table, page or dependency follows the relevant file's
   rules from the first line of code. Security, legal and speed are not
   cleanup passes.
5. When the agent is unsure which rule applies, it asks instead of
   guessing; a wrong guess in auth, payments or legal is expensive.

## How this file gets read

The user's global AI-tool configuration points agents at this index
(search the repo for `DOCS-INDEX.md` in whatever folder the project
keeps it: repo root, `docs/` or similar). Reading the relevant file
before the related work is part of the definition of done, not an extra.

Basis: distilled for this repository family from the Pre-Launch Security
Toolkit, The App Launch Checklist, the App-Performance Toolkit, the App
Legal Guard PRD and the docs-model video.
