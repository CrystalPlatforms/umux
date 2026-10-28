# umux v2.7.0 — PRD (Landing-page rebuild + full product documentation)

**Status:** planned — **new scope, discovered 2026-10-27** (/ask discovery with the PO: 23 questions in 3 series; /blueprint follow-up 2026-10-28: 6 questions; every decision confirmed by the PO).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Depends on:** nothing app-side — this release touches **only the landing app**. The docs *content* describes the product as it stands at v2.7, so in roadmap order umux Terminal (v1.9.0) and the live CLI (v1.8.0) have shipped by then and get documented as real behavior.

## Problem Statement

umux has shipped through v1.7.5 with the roadmap reaching v2.6, but it has **no product documentation**: a new user learns from the README (install/build-focused), GitHub issues, and trial and error. As the product grows (Storestation, Core, Terminal, live CLI), "check the README" stops scaling — for humans and for the LLMs users increasingly ask instead of reading. The landing itself is a single hand-written HTML file from the v1.0 era: no docs, no search, no dark mode, and every content edit is manual HTML work. There is also no machine-friendly way to hand umux's knowledge to an AI assistant.

## Solution

v2.7 rebuilds the landing as a **Vite + React app** in the Crystal theme (light/dark), keeps today's marketing sections refreshed to the v2.7 state, and adds a **complete, searchable documentation set at `/docs`** with four sections — **Desktop, Terminal, CLI, Bugs**:

- every feature documented in English with its **exact UI location** (no screenshots); Bugs = known issues + workarounds + FAQ + a GitHub Issues link;
- the PO's **branched-tree sidebar** ported 1:1 (animated SVG branches, active marker), breadcrumbs, prev/next, mobile top-dropdown;
- **search** as a magnifier icon opening a command-palette-style modal with results under the input (Pagefind, no third-party branding);
- per-doc **"Copy for AI"** — clipboard copy + `.md` download from the same single-source markdown file; raw `.md` served as static assets plus a generated `llms.txt`;
- docs always describe the **latest released version** and are refreshed at every release.

## User Stories

*(stories #149–#162 are new — added 2026-10-28)*

- **149.** As a visitor, I want the landing rebuilt as a modern React app with the Crystal theme and a light/dark toggle that follows my system preference by default, so the storefront looks current and comfortable day and night.
- **150.** As a visitor, I want the landing's existing sections kept and refreshed to the v2.7 state (hero, feature cards, install-the-CLI, first-run, roadmap, CTA) plus a Docs entry in the navigation, so the page stays the accurate front door.
- **151.** As a visitor, I want page visits still counted by GoatCounter after the rebuild, so the PO keeps his traffic picture without new tooling.
- **152.** As a docs reader, I want a `/docs` overview with a card per section (Desktop / Terminal / CLI / Bugs), so I can orient before diving in.
- **153.** As a docs reader, I want stable per-doc URLs (`/docs/<section>/<doc>`), so I can bookmark and share exact topics.
- **154.** As a docs reader, I want the branched-tree sidebar (animated branches, active marker) listing sections and their documents, so I always know where I am and can jump in one click.
- **155.** As a docs reader, I want breadcrumbs and prev/next links on every doc page, so I can traverse the docs linearly without returning to the overview.
- **156.** As a docs reader on a phone, I want the sidebar replaced by a top dropdown (sections → documents), so the docs stay usable on narrow screens.
- **157.** As a docs reader, I want search via a magnifier icon that opens a command-palette-style modal with results listed under the input, so I find topics without browsing.
- **158.** As a docs reader, I want complete coverage — every Desktop/Terminal/CLI feature documented in English with its exact UI location and no screenshots — so I never wonder where a thing lives or what it does.
- **159.** As a docs reader hitting a problem, I want a Bugs section with known issues, workarounds, an FAQ, and a link to GitHub Issues, so I can self-solve or report cleanly.
- **160.** As an AI-assisted user, I want a "Copy for AI" control on every doc page that copies the document's markdown and offers the `.md` download, so I can hand exact instructions to my LLM.
- **161.** As an agent/LLM user, I want the raw `.md` files served as static assets plus a generated `llms.txt` index at the site root, so machines can consume the docs directly.
- **162.** As the PO, I want the docs always to describe the latest released version and to be refreshed as part of every release flow, so readers never see stale instructions.

## Implementation Decisions

- **Stack (PO, Q5):** Vite + React + TypeScript + shadcn/ui; **Tabler** icons; the Crystal theme tokens (from the PO's `crs-theme.css`) ported through **tweakcn** — light and dark palettes, ABeeZee / IBM Plex Mono, green accent. Dark mode via a class strategy: default follows `prefers-color-scheme`, the visitor's choice persists.
- **Rendering (PO, Q24 = SSG):** every route pre-renders to real HTML at build; React hydrates afterwards. Reason: Pagefind indexes static HTML, and docs stay readable and SEO-able without JavaScript.
- **Docs single source (PO, Q21/Q14):** one markdown file per document inside the landing app. The same file is (a) rendered into the page at build and (b) served verbatim as a static asset (the download URL). Document metadata (title, section, order, description) lives in a separate manifest — **not frontmatter** — so a copied file is clean prose for LLMs.
- **Deep modules:** *DocsSource* — manifest + loader behind a narrow interface (ordered sections → documents with slug, title, raw markdown), validating integrity (unique slugs, non-empty files); *MarkdownRenderer* — markdown → renderable content with heading anchors and GFM (tables, fenced code). Both testable without a browser.
- **Copy for AI (PO, Q7/Q14):** per-doc control with two actions — copy the markdown to the clipboard and download the `.md` (the same served asset). One document = one `.md`.
- **Search (PO, Q12/Q26):** **Pagefind** builds its index from the pre-rendered HTML at each build; UI = a magnifier icon in the docs header opening a command-palette-style modal, results under the input; styled to the shadcn theme, no third-party branding.
- **Docs IA (PO, Q6/Q19/Q18/Q23/Q25):** `/docs` overview with section cards; per-doc routes; the branched-tree sidebar ported 1:1 from the PO's component (Hugeicons swapped for Tabler); breadcrumbs + prev/next derived from manifest order; mobile = top dropdown replacing the sidebar.
- **Docs content rules (PO, Q3/Q4/Q8/Q16/Q18):** English; complete coverage ("WSZYSTKO"); no screenshots — every feature documented with its exact UI location instead; Bugs = known issues + workarounds + FAQ + GitHub Issues link; docs describe the real product as of v2.7; always the **latest released version**.
- **`llms.txt` (PO, Q27):** generated at build from the docs manifest (title + URL per document) at the site root.
- **Marketing content (PO, Q17/Q20/Q28):** the current sections ported and refreshed to the v2.7 state; the existing content guard tests (the no-telemetry copy rule and friends) ported to the new app; the GoatCounter snippet carried over unchanged.
- **Deploy (PO, Q11):** Cloudflare Pages switches from direct file serving to build mode (build command + output directory) — the PO flips the dashboard switch at release time. The standing landing gate holds: every edit is previewed to the PO before push to `main`.
- **Maintenance (PO, Q10):** the docs refresh becomes part of the release flow; the **release-product skill gains the docs-update step when v2.7 enters carve/dispatch** (recorded in the project CLAUDE.md, 2026-10-27).

## Assumptions

- At v2.7 build time, umux Terminal (v1.9.0) and the live CLI (v1.8.0) are shipped and stable enough that documenting them as real behavior is honest.
- The master + per-version PRDs and the running app fully describe the product — no external research is needed to reach "complete coverage".
- Pagefind's build-time static index stays adequate at this docs scale (a few dozen documents).
- shadcn/ui, Tabler icons, the markdown toolchain, and Pagefind stay free/open-source (zero-cost policy).
- The GoatCounter site keeps working across the rebuild (same site id).
- The existing brand assets (logo, OG image, marketing screenshots) carry over unchanged.
- Docs describing the *latest release* satisfies readers — nobody needs historical docs.

## Tradeoffs Considered

- **Algolia DocSearch** — rejected: the free tier requires visible Algolia branding; the PO wants none. Pagefind adds no service and no logo.
- **Pure client-rendered SPA** — rejected: Pagefind would have nothing to index, no SEO, slower first paint.
- **MDX for docs** — rejected: JSX inside the files would pollute the markdown handed to LLMs.
- **Frontmatter metadata inside doc files** — rejected: metadata would travel into the copied `.md`; a separate manifest keeps pasted docs clean.
- **Hugeicons (as in the original sidebar component)** — rejected: the PO chose Tabler icons (Q5).
- **Screenshots in docs** — rejected (PO, Q16): text with exact UI locations; images rot faster than words.
- **Docs version switcher** — rejected (PO, Q22): one live version, always the latest release.
- **Separate docs subdomain / second app** — rejected: one landing app keeps the deploy and review flow simple.
- **Keeping the static hand-written HTML and bolting docs on** — rejected (PO, Q1/Q5): the React + shadcn rebuild is the point of v2.7.

## Validation Strategy

*(focus per PO Q29: the docs pipeline and Copy for AI get hard automated criteria; everything else is validated by observable artifacts and HITL)*

- **DocsSource / manifest integrity (automated):** a test asserts every manifest entry resolves to a non-empty markdown file, slugs are unique, section order matches the sidebar, all four sections exist, and every manifest route resolves in the built output. A render smoke test per document asserts heading anchors are generated and no raw markdown leaks into the HTML; the Bugs documents contain the FAQ and the GitHub Issues link.
- **Copy for AI (automated):** for every document, the copied string equals the raw file byte-for-byte; the download URL on the built site returns identical bytes; `llms.txt` lists every document with a URL that resolves in the built output.
- **Coverage "WSZYSTKO" (#158, HITL):** Adam reads the docs against the running app, section by section, and confirms every feature has an entry with the correct UI location; the per-version PRDs serve as the completeness checklist.
- **Search (#157, HITL):** the magnifier opens the palette; queries like "sleep", "notification", "import" land on the right documents.
- **Navigation (#152–#156, HITL + preview):** the overview cards, per-doc URLs, breadcrumbs, and prev/next follow the manifest order; at phone width the dropdown replaces the sidebar.
- **Theme (#149, HITL):** the toggle switches light/dark, persists across reloads, and defaults to the system preference on first visit.
- **Marketing content (#150, automated + HITL):** the ported guard tests pass (no telemetry copy etc.); Adam accepts the refreshed sections in a local preview (standing gate) before push.
- **Analytics (#151, observable):** after deploy, visits appear in the GoatCounter dashboard as before.
- **Maintenance (#162, process):** at the first release after v2.7, the docs diff shows the release's changes reflected — the release flow includes the docs refresh.
- **Deploy (HITL, PO):** the CF Pages build-mode switch happens at release; a local preview of the built output precedes the push.

## Out of Scope

- Docs version switcher (one live version).
- Screenshots or video in the docs.
- Any app-side changes — v2.7 touches only the landing; the docs describe, not change, the product.
- Comments, auth, or reader accounts on the docs (static site).
- Ecosystem surfaces (Bridge / PWA / NativeApps) — `development` branch.
- Content marketing (posts/articles) — the separate WRITES flow.

## Further Notes

- The sidebar component comes from the PO's own asset (`crs-assets/sidedocs.tsx`); the port keeps its visual behavior 1:1, only the icon set changes.
- Standing reminder (project CLAUDE.md, 2026-10-27): at v2.7 carve/dispatch time, update the release-product skill so every release also refreshes the docs.
- Until v2.7, the current landing keeps receiving normal updates; it is replaced wholesale at release.
