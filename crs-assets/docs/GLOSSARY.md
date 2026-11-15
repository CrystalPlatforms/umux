# GLOSSARY.md

One-line definitions for the terms used across this folder. Any AI coding
agent working on this repository checks this file when a term is unclear
instead of guessing what it means in this project's rules.

## Security and access

- Allowlist — a list of explicitly permitted values; anything not listed is rejected.
- Authentication — proving who a user is (login).
- Authorization — deciding what an authenticated user may do.
- Bearer token — a token sent in the Authorization header; whoever holds it is authenticated.
- Constant-time compare — a comparison taking the same time regardless of input, preventing timing leaks when checking hashes or secrets.
- CORS — browser rules deciding which other sites may read the API's responses; not authentication.
- CSP (Content-Security-Policy) — a header limiting which scripts and resources a page may load.
- CSRF — another site making a user's browser perform an action on the app using the user's cookies.
- Directory listing — a server feature showing all files in a folder; must be off.
- Hashing — one-way scrambling so passwords can be checked, never read back.
- HSTS — a header forcing browsers to always use HTTPS for the domain.
- IDOR — insecure direct object reference: changing an ID in a request to reach someone else's data.
- Idempotency key — a client-provided key letting a server safely deduplicate repeated requests (payments, retries).
- JWT — a signed token carrying user info; the signature must be verified and the contents are readable by anyone.
- Lockfile — the committed file pinning exact dependency versions; CI installs from it.
- Magic link — an emailed one-time login link; follows the same token rules as password reset.
- MFA — multi-factor authentication: a second proof (app code, passkey) beyond the password.
- Open redirect — a redirect parameter accepting attacker-controlled URLs, used for phishing.
- Parameterized query — query text fixed in advance with values sent separately; the only safe way to include user input.
- Path traversal — a filename like `../../` escaping its folder.
- Pre-signed URL — a time-limited signed URL granting temporary access to a private storage object.
- Prompt injection — hostile instructions hidden in user content that hijack an AI model.
- Publishable vs secret key — publishable keys are designed to be public; secret keys never leave the server.
- Rate limiting — capping requests per user or IP per time window; 429 means "slow down".
- RLS (Row Level Security) — database rules limiting which rows each database user can see or change.
- Rotation — replacing a credential, deploying the replacement, then revoking the old one.
- SBOM — software bill of materials: every dependency with version and licence.
- SECURITY DEFINER — a function running with its owner's rights; it bypasses RLS and must check permissions itself.
- security_invoker — a view attribute making the view apply the querying user's RLS instead of the view owner's.
- Service-role key — the database key that bypasses RLS; server-only.
- Session fixation — forcing a known session ID onto a victim; prevented by regenerating the session at login.
- Session/cookie — the token keeping a user logged in; whoever holds it is the user.
- Slopsquatting — malware published under a package name an AI tool hallucinated.
- Source map — a file mapping built code back to source; leaks original code if served publicly.
- SSRF — tricking the server into fetching internal or private URLs.
- SQL injection — user input changing a query's structure.
- Typosquatting — a malicious package named almost identically to a real one.
- user_metadata vs app_metadata — user_metadata is editable by the user and never trusted for roles; app_metadata is server-only.
- Webhook — a server-to-server notification such as "payment succeeded"; must be signature-verified.
- WITH CHECK — the RLS clause validating rows a user writes, not just reads.
- XSS — attacker script running in users' browsers.
- k-anonymity — a privacy technique (used by the Have I Been Pwned range API) where only a hash prefix is sent.

## Privacy and legal

- Adequacy decision — an EU decision that a country's data protection is equivalent, allowing transfers without extra safeguards.
- CAN-SPAM — US law for commercial email: unsubscribe, honest headers, postal address, opt-out processed within 10 business days.
- CCPA/CPRA — California privacy law: rights to know, delete, correct; opt-out of sale or sharing.
- CalOPPA — California law requiring a conspicuous privacy policy and Do-Not-Track disclosure.
- Controller / processor — the controller decides how personal data is used; the processor acts on the controller's instructions.
- COPPA — the US rule for children under 13: verifiable parental consent before collecting data.
- Dark pattern — interface design that tricks users into choices they would not make.
- Confirmshaming — guilt-based decline copy ("No thanks, I hate saving money").
- Data minimisation — collecting only the personal data a defined purpose needs.
- DPA (data processing agreement) — the contract between controller and processor required by GDPR Art. 28.
- DMCA safe harbor — US protection from copyright liability for hosting user content, conditional on a registered agent, a takedown workflow and a repeat-infringer policy.
- Drip pricing — advertising a low price and adding mandatory fees later in checkout.
- EAA (European Accessibility Act) — EU law applying accessibility requirements to covered products and services since 28 June 2025.
- ePrivacy / PECR — the EU directive and UK rules requiring consent for non-essential device storage and for electronic marketing.
- FTC — the US Federal Trade Commission; enforces against dark patterns, fake reviews, unsupported claims and hidden fees.
- GDPR — the EU General Data Protection Regulation: lawful basis, rights, retention, 72-hour breach notification.
- GPC (Global Privacy Control) — a browser signal expressing opt-out of sale or sharing; honoured where law requires.
- Impressum — the German legal-imprint page with business contact details.
- Lawful basis — the GDPR ground (consent, contract, legitimate interest) justifying each processing purpose.
- Personal data — any information relating to an identifiable person, including IP addresses and device identifiers.
- Puffery — subjective marketing praise needing no proof, unlike factual claims.
- ROSCA — the US law on negative-option subscriptions: clear disclosure and express consent before charging.
- SCC (standard contractual clauses) — EU-approved contract terms legalising personal-data transfers.
- Soft opt-in — the narrow EU exception allowing marketing to existing customers who bought similar products and were given a chance to refuse.
- SOC 2 — an independent attestation report on security controls (Type 1 point-in-time, Type 2 over a period); not a certification.
- Subprocessor — a vendor engaged by a processor; requires controller authorisation and change notification.
- Suppression list — the never-email list that survives imports and tool syncs.
- TCPA — the US law requiring consent for marketing texts and calls.
- UCPD / DMCCA — the EU unfair-commercial-practices directive and its UK counterpart banning fake urgency, fake reviews and hidden fees.
- WCAG 2.2 AA — the web accessibility standard: contrast 4.5:1, alt text, keyboard access.
- Withdrawal right — EU consumers' 14-day right to cancel distance contracts.
- 72-hour rule — the GDPR deadline for notifying the regulator about a personal-data breach.

## Performance and delivery

- Brotli / gzip — HTTP compression for text responses; Brotli is smaller, gzip is the fallback.
- Cache-aside — the pattern: check the cache, on miss query the database, store with a TTL, invalidate on write.
- CDN — edge servers caching static files near users.
- CLS (Cumulative Layout Shift) — how much the page jumps while loading; good is 0.1 or less.
- Code splitting — shipping each page only the JavaScript it needs.
- Connection pooling — reusing a small set of database connections instead of one per request.
- Core Web Vitals — Google's user-experience metrics: LCP, INP, CLS.
- Cursor/keyset pagination — paging by a cursor over an indexed column instead of OFFSET; stable for large tables.
- EXPLAIN ANALYZE — the Postgres command showing a query's plan and timing; used to prove an index helped.
- fetchpriority="high" — the HTML attribute telling the browser to load the hero image early.
- INP (Interaction to Next Paint) — how fast the page responds to taps and clicks; good is 200 ms or less.
- LCP (Largest Contentful Paint) — how fast the main content appears; good is 2.5 s or less.
- Lighthouse — Chrome's lab audit scoring a page; measures LCP and CLS, not INP.
- N+1 query — one query for a list plus one per item; fixed with a JOIN or batching.
- OpenTelemetry — the open standard for request tracing; spans time each step of a request.
- Overscan — extra rows rendered beyond the visible window in a virtualized list.
- p75 — the 75th percentile: the value 75% of real users experience; how Core Web Vitals are judged.
- RUM (real-user monitoring) — measuring speed from real users' devices rather than the lab.
- Sampling — recording only a fraction of traces in production to control volume and cost.
- srcset / sizes — HTML attributes serving differently sized images per screen.
- TBT (Total Blocking Time) — the lab proxy for INP: how long the main thread stays blocked.
- TTL — time-to-live: how long a cache entry stays valid.
- Virtualization — rendering only the visible list rows plus overscan regardless of list length.
- WebP / AVIF — modern image formats, much smaller than JPEG or PNG at the same quality.

Basis: glossary of the Pre-Launch Security Toolkit; definitions aligned
with how this folder's files use each term.
