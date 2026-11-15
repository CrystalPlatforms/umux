# SEO.md

Search engine and share-preview rules for this project. Any AI coding agent
working on this repository follows this file. Human developers follow it too.

Page-speed rules live in `PERFORMANCE.md`; third-party script loading and
security headers in `SECURITY.md`.

## Rules that always apply

1. Every indexable page has a unique `<title>` and a unique meta description.
   No duplicates across routes, no templates left unfilled.
2. Every page ships the three Open Graph tags: `og:image`, `og:title`,
   `og:description`. Image. Title. Description — that is the whole share card.
3. A valid XML sitemap exists at `/sitemap.xml` and is regenerated whenever
   routes change.
4. Production pages are verified in Google Search Console and the sitemap is
   submitted there once.
5. Nothing that should be private is indexable: admin panels, dashboards,
   API routes and anything behind login are excluded from indexing.

## Page title

- One `<title>` per page, 50–60 characters. Longer titles get truncated in
  results.
- Put the page's main keyword near the front, brand name at the end:
  `Free Website Templates - Website Design ... - Strikingly`.
- Format: `<Page topic> | <Brand>` or `<Page topic> - <Brand>`.
- The title must describe what is actually on the page. No keyword stuffing,
  no ALL CAPS, no "Welcome to".

## Meta description

- 150–160 characters. Google shows it under the page title in results and may
  rewrite it, but a well-written one is used most of the time.
- Write it as ad copy for the page: what the user gets, in plain words.
  Example: "Get inspired by website templates, profiles, and businesses that
  thousands have launched on Strikingly. Choose a template for your website."
- One per page, matching that page — not the homepage description copied
  everywhere.

## Open Graph tags (share previews)

The minimum complete set for link previews in messengers and social feeds:

```html
<meta property="og:image" content="https://example.com/og/home.png" />
<meta property="og:title" content="YourBrand | Modern Home Interiors" />
<meta property="og:description" content="Design your dream home. Free consultation. On-time delivery." />
```

- `og:image`: 1200×630 px, under ~1 MB, readable at thumbnail size. Every
  important page gets its own image when it makes sense; the brand image is
  the fallback.
- `og:title` / `og:description`: may match the meta title/description but can
  be shorter and punchier — they are read in a feed, not in search results.
- Optional additions when needed: `og:url` (canonical URL), `og:type`
  (`website` by default, `article` for posts), `twitter:card`
  (`summary_large_image`) so X renders the large preview.

## XML sitemap

- Live at `/sitemap.xml`, valid XML, contains every canonical, indexable URL.
- Regenerate it as part of the build or on publish — never edit by hand.
- Keep it under 50,000 URLs / 50 MB per file; beyond that, split with a
  sitemap index.
- Reference it in `robots.txt`:
  ```
  User-agent: *
  Allow: /

  Sitemap: https://example.com/sitemap.xml
  ```

## robots.txt and indexing control

- Non-indexable routes (admin, account, API, checkout steps) either return
  `X-Robots-Tag: noindex` or are disallowed in `robots.txt`. Prefer `noindex`
  for pages that might still get linked.
- Behind-login pages do not need blocking — they are invisible to crawlers
  anyway — but mark them `noindex` if they can be reached by URL.
- Never `Disallow:` a page and rely on that alone to hide it; use `noindex`
  if the page must not appear in results.

## Google Search Console

One-time setup, then light monitoring:

1. Verify the production domain as a property (DNS verification for the whole
   domain).
2. Submit `https://<domain>/sitemap.xml` under Sitemaps.
3. After launches: check Coverage / Pages for new errors (404s, noindex on
   pages that should index, redirects chains).
4. Use Performance to confirm pages actually appear in search; use URL
   Inspection to check a single live URL.

## How to verify

- View source on every new page: title, meta description, and the three OG
  tags present and page-specific.
- Paste a URL into a preview checker (e.g. metatags.io, Facebook Sharing
  Debugger, X Card Validator) and confirm the card renders image + title +
  description.
- Open `/sitemap.xml` in the browser — it parses, and every listed URL
  returns 200.
- Search Console shows the sitemap as "Success" with discovered URLs.
- Spot-check SEO, speed and security practices together with checkbot.io or
  seotest.me (free crawls).

Basis: the docs-model video's SEO reel (Search Console, XML sitemap,
title/meta, OG tags) and standard practice derived from them.
