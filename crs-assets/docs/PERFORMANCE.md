# PERFORMANCE.md

Speed rules for this project. Any AI coding agent working on this
repository follows this file.

## Budgets (hard limits, checked on every change)

| Metric | Limit |
|---|---|
| LCP (p75, real users) | <= 2.5 s |
| INP (p75, real users) | <= 200 ms |
| CLS | <= 0.1 |
| TTFB | <= 0.8 s |
| Lighthouse mobile | >= 90 |
| Initial JS per page | <= 200 KB gzipped |
| Hero image | <= 150 KB |
| First-load page weight | <= 1.5 MB |
| Fonts | <= 3 woff2 files, <= 200 KB total |
| API p95 response time | <= 500 ms |
| DB queries per request | <= 5; zero N+1 |

The safe change loop: measure first, change one thing, re-measure. Never
optimize without a baseline.

## Measure before fixing

- Pick the 5 most important pages and 5 most-used endpoints.
- Per page (mobile lab audit with throttling): LCP, CLS, Total Blocking
  Time, total JS size, image weight, request count. Lab audits do not
  measure INP; take INP from real-user data or mark it "not measured".
- Per endpoint: average and slowest response time, database queries per
  request.
- Record results as a "Before" table; add an "After" column after each
  fix. Keep it in this file or the project wiki.
- Rank fixes by impact on real users versus effort. The quick wins below
  come first.

## Quick wins (high impact, low effort)

1. WebP or AVIF images at the right size (quality 75-80)
2. Static files through a CDN (most hosts include one)
3. Timeouts on every external API call
4. Independent requests run in parallel
5. Database indexes on common queries
6. Servers and database in the same region
7. Third-party scripts loaded after the page
8. Connection pooling
9. Real-user monitoring (Core Web Vitals)
10. Testing on slow phones and networks

## Rules by area

### Frontend

- Skeleton screens match the final layout's size and position; swap out
  the moment data arrives; `aria-busy="true"` on the loading region;
  dimensions wrong cause layout shift; disable shimmer under
  `prefers-reduced-motion`.
- Optimistic UI only for small, low-risk actions (likes, toggles,
  rename, reorder): update local state, send in background, roll back on
  failure. Never for payments, deletions or security-sensitive actions.
  The server stays the source of truth.
- Server-render or build-time-render pages that rarely change; client
  render only for interactive logged-in screens. Users see content much
  sooner and crawlers can read the page. No secrets or other users' data
  in server-rendered HTML.
- Third-party scripts (analytics, chat, ads, A/B): list them, remove the
  unused, load the rest with `defer` or `async`; heavy embeds become a
  placeholder that loads on click or scroll-into-view.
- Code splitting: route-based as the baseline; lazy-load heavy components
  (charts, editors, maps, modals, admin screens) with dynamic `import()`
  and a loading placeholder; move heavy imports out of shared layouts.
  Too many tiny chunks add requests; split the big and rare.
- Virtualize lists that can hold hundreds of rows: render visible rows
  plus a small overscan, keep selection, keyboard and accessibility
  working; combine with pagination so data is not all fetched either.
  Short lists stay alone.
- Re-renders: fix what the profiler flags. State stored too high, large
  fast-changing contexts, new objects per render, unstable keys. Move
  state closer to use, split contexts, memoize genuinely expensive parts.
  Do not memoize everything.

### Assets

- Images: modern format (AVIF, then WebP) at the real display size,
  quality 75-80, width/height or aspect-ratio set, `srcset`/`sizes` so
  phones do not download desktop files. Never a 4000 px photo in a
  400 px slot.
- Lazy-load below-the-fold images and iframes (`loading="lazy"`);
  videos get `preload="none"` with a poster. The hero image is never
  lazy: it gets `fetchpriority="high"`.
- Fonts: self-hosted WOFF2 from this domain, 2-3 weights or one variable
  file, `font-display: swap`, preload only the primary file, matching
  fallback stack. Check the licence allows self-hosting. Self-hosting
  also removes the third-party IP leak (`LEGAL.md`).
- Dependencies: run a dep-checker and a bundle analyzer; list the 10
  heaviest; replace with lighter libraries or native features
  (`Intl` for dates) where behaviour stays identical; switch to specific
  imports where it shrinks the bundle. Check config-file and script usage
  before deleting anything. Build and test after.

### Network

- No duplicate API calls: share one in-flight request, cache results,
  disable submit buttons while pending, idempotency keys on create and
  payment endpoints. React StrictMode double-effects in dev are normal;
  duplicates in production are not.
- Debounce search and autocomplete at ~300 ms (250-400 range), cancel the
  previous in-flight request (AbortController) so stale results never
  overwrite newer ones, skip queries under 2 characters.
- Compression: Brotli preferred with gzip fallback for HTML, CSS, JS,
  JSON, SVG; pre-compress static assets at build time; exclude
  already-compressed formats; never buffer a streaming response.
- Browser caching: versioned/hashed static files get
  `Cache-Control: public, max-age=31536000, immutable`; HTML gets
  `no-cache`; user-specific responses `private`; sensitive responses
  `no-store`. Only long-cache files whose names change with content.
- Never let a CDN cache personalized or logged-in responses: one user
  may see another user's page (`SECURITY.md`).

### Backend

- Paginate every list endpoint: cursor-based (keyset) preferred, default
  page 20, server-side maximum 100, ordering column indexed. Breaking
  change: keep a backward-compatible default when consumers exist.
- Slow tasks (email, PDF, video, bulk AI) go to a background job queue:
  the request returns 202 with a job ID immediately; workers retry with
  exponential backoff, a max attempt count and a timeout; jobs are
  idempotent; the frontend can poll status or get notified. Serverless
  post-response work can be killed; use a real queue.
- Timeouts on every outbound call (`AbortSignal.timeout()` or
  equivalent), through one shared helper: a few seconds for quick
  lookups, longer for AI generation and file processing. On timeout,
  return a clear error or fallback. Retry only safe, idempotent calls,
  never payments without provider idempotency keys.
- Parallelize independent work with `Promise.all` (or `allSettled` when
  partial failure is fine); fix request waterfalls by starting fetches
  at the page level; cap concurrency for large loops. Dependent calls
  stay sequential.
- Cache frequently-read, rarely-changing data (settings, product lists,
  public profiles, counts) with cache-aside and a TTL: check cache, on
  miss query the DB, store, invalidate on write. User-specific cache
  keys include the user ID; a shared key leaks data between users
  (`SECURITY.md`). Fall back to the database when the cache is down;
  never cache secrets.

### Database

- Indexes on WHERE, JOIN and ORDER BY columns, especially foreign keys
  and lookup columns (email, slug, user_id, created_at); composite
  indexes in the right column order. Every index slows writes and costs
  storage, so skip duplicates and nice-to-haves. Ship as a migration;
  on large production tables use `CREATE INDEX CONCURRENTLY`. Verify
  with `EXPLAIN ANALYZE` before and after.
- N+1 queries: enable query logging, find calls inside loops and
  resolvers, fix with a JOIN, eager loading or batching, loading only
  the relations the page uses. Query count stays flat whether the list
  has 10 or 10,000 rows.
- Select only needed columns, starting with large text/JSON/binary on
  list pages. Bonus: sensitive fields (password hashes, tokens, internal
  notes) never reach responses (`SECURITY.md`).
- Connection pooling: one shared client per server process, never a new
  client per request; on serverless use the provider's pooled URL and
  mind transaction-mode limitations (prepared statements); pool size
  safely under the database's connection limit, read from env.

### AI

- Stream responses token by token via SSE or a streamed response;
  render markdown as it arrives; Stop button aborts the upstream
  request; stop on cancel or page exit. Confirm nothing buffers the
  stream (proxies, compression, hosting). Measure time-to-first-token
  before and after.

### Infra

- Static through the host's CDN before adding a provider; content-hashed
  file names; confirm HTML and API responses are not cached; learn to
  read cache-status headers.
- Servers, database and worker hosts in the same region; move
  database-heavy edge functions next to the database; for global users,
  CDN for static plus read replicas for reads. Ten queries across an
  ocean can add hundreds of milliseconds.
- Load balancing only when actually needed: stateless app (sessions,
  uploads, in-memory caches and counters move to shared storage), a
  `/health` endpoint checking the DB connection, scheduled jobs that
  cannot run twice, minimum and maximum instance count set so a spike
  cannot run up the bill.

### Monitoring

- Real-user monitoring collecting Core Web Vitals (LCP, INP, CLS) plus
  load time, tagged by route, device type and connection type; API
  response times if possible; consent respected; no personal data in
  performance events. Check what the host already provides before
  adding a service.
- Tracing (OpenTelemetry or the host's built-in) with automatic
  instrumentation for HTTP, DB and outbound calls; custom spans around
  business-critical steps; sampled in production; no secrets or personal
  data in span attributes. The slowest step of a request should be
  findable in one minute.
- Test the way users actually run the app: DevTools throttling at CPU
  4x-6x slowdown with slow 4G/3G, and at least one real mid-range or
  budget Android phone. Emulation is an approximation; real devices
  reveal what DevTools hides.

## Security guardrails while optimizing

- Never cache user-specific data under a shared key; never let CDN or
  browser cache private responses.
- No secrets or other users' data in server-rendered HTML.
- No blind retries on payment calls.
- Compression and streaming: never buffer streaming responses.
- Caches, CDNs and background queues introduce data leaks and duplicate
  charges when done carelessly; the security sections of `SECURITY.md`
  apply to every optimization here.

## Verify

- Re-run the "Before" table measurements as the "After" column, same
  pages and settings.
- Lighthouse mobile >= 90 on key pages; checkbot.io for combined
  SEO/speed/security spot checks; WebPageTest for regional views.
- RUM dashboards: Core Web Vitals at p75 per page, worst pages first.
- DevTools Network: no duplicate calls; text responses compressed;
  static files served from cache; images in modern formats at the right
  sizes.
- Full test suite, build, and a click-through of main flows after each
  optimization group.

Basis: App-Performance Toolkit (31 fixes with verify steps and targets).
