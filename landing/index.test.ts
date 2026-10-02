import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";

// Issue #35 (v1.0 Phase 11) — landing page.
//
// The public interface under test is `landing/index.html` exactly as
// Cloudflare Pages will serve it (Pages root = landing/, no build step).
// Same disk-read pattern as Readme.test.ts: read the real file so the
// assertions are deterministic regardless of any test-runner transformation.
//
// Assumptions encoded here (stated before RED):
//  - Input: the full HTML text of landing/index.html at the repo root.
//  - "Auto-latest" downloads use the GitHub Releases pattern
//    `releases/latest/download/<asset>` against VERSION-LESS asset names.
//    The release workflow (.github/workflows/release.yml) uploads those
//    permanent aliases alongside the versioned files on every release, so
//    the page never needs a version bump again. Windows ships NSIS only
//    (.exe, no .msi).
//  - Badges are dynamic shields.io endpoints (they update themselves); we
//    assert the endpoint shape, not the current numbers.
//  - Media (demo GIF, screenshots) live under landing/assets/ and ship with
//    an inline-SVG onerror fallback until real files are dropped in.
//  - GoatCounter is ACTIVE (account `crystalstudio`, 2026-08-27) and is the
//    page's single external script; the UI logic itself stays inline.
//  - #91 (v1.7.0 Phase 9) added a roadmap block; the docs and ecosystem
//    teaser sections proposed in the same phase were REMOVED at Adam's
//    direction (2026-09-30) — they cluttered the page. The roadmap carries
//    every released version and every planned one, one short line each
//    (mirroring the 2026-09-12 renumbered master-PRD roadmap: Storestation
//    v1.7.0 is current; Core Always-On v1.7.5, live CLI v1.8.0, TUI v1.9.0,
//    Agents View v2.0.0, …, Ecosystem v3.0.0 planned). Internal anchors
//    must resolve to real ids; "no dead external links" is asserted as a
//    destination WHITELIST (we can't fetch the network from a unit test —
//    a link to an unknown host/path shape is the failure mode worth
//    catching here).
//  - Boundary: intentionally NOT tested — the real Cloudflare deployment
//    (HITL by Adam), visual appearance, shields.io uptime, and whether the
//    actual GIF/screenshots exist yet.

const here = path.dirname(fileURLToPath(import.meta.url));
const html = fs.readFileSync(path.join(here, "index.html"), "utf8");

// Parse the served document fresh per test, like a browser would.
const parse = (): Document => new DOMParser().parseFromString(html, "text/html");

// A document with the page's inline UI script executed, for click-behavior
// tests. url is set so relative asset paths resolve like on umux.pages.dev.
const pageWithScripts = (): Document =>
    new JSDOM(html, {
        runScripts: "dangerously",
        url: "https://umux.pages.dev/",
    }).window.document;

describe("umux landing page (Issue #35, Phase 11)", () => {
    it("serves a mobile-ready HTML document", () => {
        // Parses as a document and carries the minimum head metadata a
        // fast, phone-friendly static page needs: a title, a description,
        // the viewport meta, and a favicon.
        const doc = parse();
        expect(doc.querySelector("title")?.textContent).toMatch(/umux/i);
        const viewport = doc.querySelector('meta[name="viewport"]');
        expect(viewport?.getAttribute("content")).toMatch(/width=device-width/);
        expect(
            doc.querySelector('meta[name="description"]')?.getAttribute("content"),
        ).toBeTruthy();
        expect(doc.querySelector('link[rel="icon"]')).toBeTruthy();
    });

    it("brands the header with the logo and the real favicon", () => {
        // Real assets live in landing/assets/ (Pages root is landing/). The
        // "❯" prompt glyph in front of the brand is replaced by the logo image.
        const doc = parse();
        const brandImg = doc.querySelector(".brand img");
        expect(brandImg?.getAttribute("src")).toBe("assets/umux-logo.png");
        expect(brandImg?.getAttribute("alt")).toMatch(/logo/i);
        expect(doc.querySelector('link[rel="icon"]')?.getAttribute("href")).toBe(
            "assets/umux-favicon.ico",
        );
        const style = doc.querySelector("style")?.textContent ?? "";
        expect(style.includes("❯"), "prompt glyph must be gone").toBe(false);
    });

    it("collapses the Linux downloads into one dropdown button", () => {
        // One "Download for Linux" button; the three package formats hide
        // behind it. No Linux file may remain outside the dropdown.
        const doc = parse();
        const dropdown = doc.querySelector("details.dropdown");
        expect(dropdown, "Linux dropdown missing").toBeTruthy();
        expect(dropdown!.querySelector("summary")?.textContent?.trim()).toBe(
            "Download for Linux",
        );
        const menuHrefs = [...dropdown!.querySelectorAll("a")].map((a) =>
            a.getAttribute("href"),
        );
        for (const asset of [
            "umux_amd64.AppImage",
            "umux_amd64.deb",
            "umux_x86_64.rpm",
        ]) {
            expect(
                menuHrefs.some((h) => h?.endsWith(asset)),
                `${asset} not reachable in the Linux dropdown`,
            ).toBe(true);
        }
        const linuxLinksOutside = [...doc.querySelectorAll("a")].filter(
            (a) =>
                /AppImage$|\.deb$|\.rpm$/.test(a.getAttribute("href") ?? "") &&
                !dropdown!.contains(a),
        );
        expect(
            linuxLinksOutside,
            "Linux package links must live inside the dropdown only",
        ).toHaveLength(0);
    });

    it("puts GitHub and website icon buttons in the header", () => {
        // Two icon shortcuts at the top: the umux repo on GitHub and the
        // studio website. Inline SVG icons (no extra network requests),
        // labelled for screen readers, opened in a new tab safely.
        const doc = parse();
        const iconBtns = [...doc.querySelectorAll("header a.icon-btn")];
        expect(iconBtns, "expected exactly two header icon buttons").toHaveLength(2);
        const gh = iconBtns.find(
            (a) => a.getAttribute("href") === "https://github.com/CrystalPlatforms/umux",
        );
        const web = iconBtns.find(
            (a) => a.getAttribute("href") === "https://crystal-studio.dev",
        );
        expect(gh?.getAttribute("aria-label") ?? "").toMatch(/github/i);
        expect(web?.getAttribute("aria-label") ?? "").toMatch(/website|crystal-studio/i);
        expect(gh?.querySelector("svg"), "GitHub icon missing").toBeTruthy();
        expect(web?.querySelector("svg"), "website icon missing").toBeTruthy();
        for (const a of [gh, web]) {
            expect(a?.getAttribute("target")).toBe("_blank");
            expect(a?.getAttribute("rel") ?? "").toContain("noopener");
        }
    });

    it("points the download buttons at the latest release assets for all three platforms", () => {
        // "Auto-latest" = the GitHub `releases/latest/download/<asset>` pattern,
        // which always resolves to the newest release. The asset names are the
        // version-less aliases the release workflow uploads to every release,
        // so no page edit is needed when a new version ships.
        const doc = parse();
        const hrefs = new Set(
            [...doc.querySelectorAll("a")].map((a) => a.getAttribute("href")),
        );
        const base = "https://github.com/CrystalPlatforms/umux/releases/latest/download";
        const assets = [
            "umux_amd64.AppImage", // Linux
            "umux_amd64.deb", // Linux
            "umux_x86_64.rpm", // Linux
            "umux_universal.dmg", // macOS (universal)
            "umux_x64-setup.exe", // Windows (NSIS only — no .msi)
        ];
        for (const asset of assets) {
            expect(
                hrefs.has(`${base}/${asset}`),
                `missing download link for ${asset}`,
            ).toBe(true);
        }
        // Fallback for anything else: the release page itself.
        expect(
            hrefs.has("https://github.com/CrystalPlatforms/umux/releases/latest"),
        ).toBe(true);
    });

    it("never hardcodes a version number in a download link or install command", () => {
        // Regression guard for the version-less-alias rework (2026-09-06):
        // versioned names like `umux_1.0.4_amd64.deb` go stale on the next
        // release and 404. Any `umux_`/`umux-` name carrying a digit sequence
        // with dots in the page (links, dialogs, anywhere) is a bug.
        expect(html).not.toMatch(/umux[-_]\d/);
    });

    it("renders auto-updating badges for version, license and downloads", () => {
        // Dynamic shields.io endpoints — the numbers update themselves on every
        // release, so no page edit is needed for a new version.
        const doc = parse();
        const badges = [...doc.querySelectorAll("img")].filter((img) =>
            (img.getAttribute("src") ?? "").startsWith("https://img.shields.io/"),
        );
        const srcs = badges.map((b) => b.getAttribute("src") ?? "");
        expect(
            srcs.some((s) =>
                s.startsWith("https://img.shields.io/github/v/release/CrystalPlatforms/umux"),
            ),
            "missing version badge",
        ).toBe(true);
        expect(
            srcs.some((s) =>
                s.startsWith("https://img.shields.io/github/license/CrystalPlatforms/umux"),
            ),
            "missing license badge",
        ).toBe(true);
        expect(
            srcs.some((s) =>
                s.startsWith("https://img.shields.io/github/downloads/CrystalPlatforms/umux/total"),
            ),
            "missing downloads badge",
        ).toBe(true);
        for (const badge of badges) {
            expect(badge.getAttribute("alt"), "badge must have alt text").toBeTruthy();
        }
    });

    it("shows the real product screenshots with a placeholder fallback", () => {
        // Every shipped-feature tile carries its own screenshot (Adam,
        // 2026-09-16): umux-first is the big hero image, and the tiles use
        // umux-workspaces / umux-terminal / umux-agent / umux-ssh /
        // umux-updates / umux-session / umux-cmux. Only the herdr tile stays
        // imageless — that feature isn't built yet, there is nothing to shoot.
        // A broken image must still degrade to the inline placeholder, never
        // a broken-icon.
        const doc = parse();
        const hero = doc.querySelector(".demo-frame img");
        expect(hero?.getAttribute("src")).toBe("assets/umux-first.png");

        const features = [...doc.querySelectorAll(".feature")];
        const byHeading = (re: RegExp) =>
            features.find((f) => re.test(f.querySelector("h2")?.textContent ?? ""));
        const expectTileImg = (re: RegExp, file: string) =>
            expect(
                byHeading(re)?.querySelector("img")?.getAttribute("src"),
                `the "${re}" tile must use assets/${file}`,
            ).toBe(`assets/${file}`);
        expectTileImg(/workspaces/i, "umux-workspaces.png");
        expectTileImg(/embedded terminal/i, "umux-terminal.png");
        expectTileImg(/agent status/i, "umux-agent.png");
        expectTileImg(/in-app updates/i, "umux-updates.png");
        expectTileImg(/session restore/i, "umux-session.png");
        expectTileImg(/import from cmux/i, "umux-cmux.png");

        // Exactly two tiles stay imageless (2026-10-02, Adam): herdr (coming
        // soon) and SSH panels — its placeholder shot was pulled, the card
        // moved to the end of the list right before herdr.
        const imgTiles = features.filter((f) => f.querySelector("img"));
        expect(imgTiles, "every tile except ssh/herdr must carry a screenshot").toHaveLength(
            features.length - 2,
        );
        expect(byHeading(/herdr/i)?.querySelector("img")).toBeNull();
        expect(byHeading(/ssh panels/i)?.querySelector("img")).toBeNull();

        const media = [hero, ...doc.querySelectorAll(".feature img")];
        for (const img of media) {
            expect(
                img?.getAttribute("onerror"),
                `${img?.getAttribute("src")} must fall back to the inline placeholder`,
            ).toContain("data:image/svg+xml");
        }
        // The per-OS screenshot strip is gone — no stale references left.
        expect(html).not.toMatch(/demo\.gif|screenshot-(linux|macos|windows)\.png/);
    });

    it("leads with the one-line pitch and the eight feature tiles", () => {
        // Hero pitch + eight tiles (2026-09-16 refresh to the v1.6.2 state):
        // workspaces & panels, embedded terminal (shipped since v0.1 — its
        // old "coming soon" badge was wrong), agent status & notifications,
        // SSH panels, in-app updates, session restore, import from cmux
        // (shipped) and import from herdr (the only still-unbuilt one).
        const doc = parse();
        expect(doc.querySelector("h1")?.textContent ?? "").toMatch(/terminal workspace/i);

        const features = [...doc.querySelectorAll(".feature")];
        expect(features, "expected eight feature tiles").toHaveLength(8);
        const text = features
            .map((f) => f.textContent?.toLowerCase() ?? "")
            .join("\n");
        expect(text).toMatch(/workspace/);
        expect(text).toMatch(/panel/);
        expect(text).toMatch(/agent status/);
        expect(text).toMatch(/notification/);
        expect(text).toMatch(/session restore/);
        expect(text).toMatch(/import from cmux/);
        expect(text).toMatch(/import from herdr/);
        expect(text).toMatch(/embedded terminal/);
        expect(text).toMatch(/ssh/);
        expect(text).toMatch(/in-app updates/);
        // Unbuilt features must say so up front on their tiles.
        const byHeading = (re: RegExp) =>
            features.find((f) => re.test(f.querySelector("h2")?.textContent ?? ""));
        expect(byHeading(/herdr/i)?.textContent ?? "").toMatch(/coming soon/i);
        // Regression guard: the embedded terminal shipped long ago (xterm.js,
        // v0.1) — the "coming soon" badge it once carried must never return.
        expect(byHeading(/embedded terminal/i)?.textContent ?? "").not.toMatch(
            /coming soon/i,
        );
    });

    it("renders link-preview meta (Open Graph / Twitter card)", () => {
        // Sharing the URL on X/Discord/LinkedIn must produce a rich card:
        // title, description and the 1200x630 preview image served from the
        // Pages domain.
        const doc = parse();
        expect(
            doc.querySelector('meta[property="og:title"]')?.getAttribute("content"),
        ).toMatch(/umux/i);
        expect(
            doc.querySelector('meta[property="og:description"]')?.getAttribute("content"),
        ).toBeTruthy();
        expect(doc.querySelector('meta[property="og:image"]')?.getAttribute("content")).toBe(
            "https://umux.pages.dev/assets/og.jpg",
        );
        expect(doc.querySelector('meta[name="twitter:card"]')?.getAttribute("content")).toBe(
            "summary_large_image",
        );
    });

    it("warns about unsigned builds and links to the install docs", () => {
        // Zero-cost policy: builds are unsigned, so macOS Gatekeeper and
        // Windows SmartScreen will prompt on first run. The page must set
        // that expectation and hand users the full README instructions.
        const doc = parse();
        const note = (doc.querySelector(".first-run")?.textContent ?? "").toLowerCase();
        expect(note).toMatch(/open anyway|right-click/); // macOS workaround
        expect(note).toMatch(/run anyway/); // Windows SmartScreen workaround
        expect(doc.querySelector(".first-run a")?.getAttribute("href")).toBe(
            "https://github.com/CrystalPlatforms/umux#installation",
        );
    });

    it("loads exactly one external script: the active GoatCounter counter", () => {
        // "Loads fast" AC: the page's own UI (dropdown, dialogs) is one small
        // inline script — the ONLY external script allowed is the GoatCounter
        // counter, activated 2026-08-27 with Adam's site code `crystalstudio`
        // (free tier: non-commercial, cookie-free, no consent banner needed).
        const doc = parse();
        const external = [...doc.querySelectorAll("script[src]")];
        expect(external).toHaveLength(1);
        const counter = external[0];
        expect(counter.getAttribute("src")).toBe("https://gc.zgo.at/count.v1.js");
        expect(counter.getAttribute("async"), "counter must not block page load").not.toBeNull();
        expect(counter.getAttribute("data-goatcounter")).toBe(
            "https://crystalstudio.goatcounter.com/count",
        );
    });

    it("thanks desktop downloaders with a centered dialog", () => {
        // Clicking Download for Windows/macOS starts the native download AND
        // opens a modal thanking the user and wishing them well ("we wish
        // … regular" copy Adam asked for).
        for (const platform of ["windows", "macos"]) {
            const doc = pageWithScripts();
            const btn = doc.querySelector(`a[data-platform="${platform}"]`);
            expect(btn, `${platform} download button missing`).toBeTruthy();
            btn!.click();
            const dlg = doc.querySelector("dialog#after-download");
            expect(dlg, "post-download dialog missing").toBeTruthy();
            expect(dlg!.getAttribute("open"), "dialog should be open").not.toBeNull();
            const text = dlg!.textContent ?? "";
            expect(text).toMatch(/thanks for downloading/i);
            // Adam's exact wording (2026-08-27):
            expect(text).toMatch(/we wish you'll have many productive sessions in umux/i);
            expect(text).toMatch(/hope you'll become a regular/i);
            expect(text).toContain("😃");
        }
    });

    it("walks Linux users through the install right after the download starts", () => {
        // Picking a package in the dropdown starts the download AND opens a
        // dialog with the install command matching that exact format.
        const cases: [string, RegExp, string][] = [
            ["AppImage", /chmod \+x/, "umux_amd64.AppImage"],
            [".deb", /apt install/, "umux_amd64.deb"],
            [".rpm", /dnf install/, "umux_x86_64.rpm"],
        ];
        for (const [format, command, file] of cases) {
            const doc = pageWithScripts();
            const link = doc.querySelector(`a[data-install="${format}"]`);
            expect(link, `${format} option missing in the dropdown`).toBeTruthy();
            link!.click();
            const dlg = doc.querySelector("dialog#after-download")!;
            expect(
                dlg.getAttribute("open"),
                `install dialog should open for ${format}`,
            ).not.toBeNull();
            const text = dlg.textContent ?? "";
            expect(text, `${format} dialog must show its install command`).toMatch(
                command,
            );
            expect(text, `${format} dialog must name the downloaded file`).toContain(
                file,
            );
        }
    });

    it("centers and enlarges the dialogs on screen", () => {
        // Regression: the global `* { margin: 0 }` reset also wiped the UA's
        // `dialog { margin: auto }`, which is what centers a modal — so
        // dialogs stuck to the corner (Adam's report, 2026-08-27). The
        // dialog rule must restore the centering itself, and be comfortably
        // large with its own scroll when taller than the screen.
        // Effective CSS only — strip /* comments */ so their text can't
        // confuse the rule extraction.
        const style = (parse().querySelector("style")?.textContent ?? "").replace(
            /\/\*[\s\S]*?\*\//g,
            "",
        );
        const rule = style.match(/dialog#after-download\s*{[^}]*}/)?.[0] ?? "";
        expect(rule, "dialog rule missing").toBeTruthy();
        expect(rule, "dialog must center itself (margin: auto)").toMatch(
            /margin:\s*auto/,
        );
        expect(rule, "dialog must scroll when taller than the viewport").toMatch(
            /max-height/,
        );
    });

    it("keeps the download buttons thumb-friendly (44px touch targets)", () => {
        // HITL AC: "Adam opens the page on his phone and can reach a download
        // in two taps" — buttons need an adequate touch target, which is the
        // one stylesheet rule this page asserts on. The rest of the CSS is
        // presentational and intentionally untested.
        const doc = parse();
        const style = doc.querySelector("style")?.textContent ?? "";
        expect(style).toMatch(/\.btn\s*{[^}]*min-height:\s*44px/);
    });

    // --- Install-the-CLI section (#65, Phase 6) -----------------------------

    it("shows the Install-the-CLI section with the curl | sh one-liner", () => {
        // The section must carry the EXACT command a visitor copies — piped
        // straight from the repo's main branch, matching where install.sh
        // actually lives. No vanity text may replace the copyable line.
        // The terminal-styled block must also expose the one-click copy button.
        const doc = parse();
        const section = doc.querySelector("section.install-cli");
        expect(section, "install-cli section missing").toBeTruthy();
        expect(section!.querySelector("h2")?.textContent).toMatch(/install the cli/i);
        const code = section!.querySelector("pre code")?.textContent ?? "";
        expect(code).toBe(
            "curl -fsSL https://raw.githubusercontent.com/CrystalPlatforms/umux/main/install.sh | sh",
        );
        expect(
            section!.querySelector("button[data-copy]"),
            "one-click copy button missing",
        ).toBeTruthy();
    });

    it("tells visitors how to preview the script before running it", () => {
        // Running piped scripts blind is bad practice — the section must show
        // the --dry-run form so the cautious path is the documented one.
        const doc = parse();
        const text = doc.querySelector("section.install-cli")?.textContent ?? "";
        expect(text).toMatch(/--dry-run/);
    });

    it("is upfront that the CLI cannot live-control a running app yet", () => {
        // The CLI manages the saved store only; live control of a running app
        // is roadmap (future version). The section must set that expectation
        // so nobody installs the CLI expecting a remote control.
        const doc = parse();
        const text = doc.querySelector("section.install-cli")?.textContent ?? "";
        expect(text).toMatch(/future version/i);
    });

    it("closes with a final call-to-action pointing at the downloads", () => {
        // After all the content, a closing band with one primary download
        // button (JS swaps it to the visitor's platform) and a GitHub link.
        const doc = parse();
        const cta = doc.querySelector(".cta");
        expect(cta, "cta section missing").toBeTruthy();
        const primary = cta!.querySelector("a.btn-primary");
        expect(primary?.getAttribute("href")).toMatch(
            /^https:\/\/github\.com\/CrystalPlatforms\/umux\/releases\/latest/,
        );
    });

    it("never mentions telemetry anywhere on the page", () => {
        // Standing rule (Adam, 2026-09-16): the word "telemetry" must not
        // appear on the landing — no claims about it in either direction.
        // The old CTA promised "no telemetry", which stopped being a claim
        // the page should make; the topic stays off the page entirely.
        expect(html.toLowerCase()).not.toContain("telemetry");
    });

    it("gates animations behind JS and honors reduced motion", () => {
        // Reveal-hiding may only apply under `html.js` (set by a tiny head
        // script), so a no-JS visitor never sees blank sections. A
        // prefers-reduced-motion block must undo the motion for people who
        // opt out of animations.
        const doc = parse();
        expect(doc.querySelector("script")?.textContent).toContain('classList.add("js")');
        const style = doc.querySelector("style")?.textContent ?? "";
        expect(style).toMatch(/\.js \.reveal\s*\{/);
        expect(style).toContain("prefers-reduced-motion: reduce");
    });
});

// --- Roadmap section (#91, v1.7.0 Phase 9; docs + ecosystem removed at
// --- Adam's direction, 2026-09-30) ----------------------------------------

describe("umux landing page — roadmap (Issue #91, Phase 9)", () => {
    it("wires the header nav to the page's own sections", () => {
        // Plain anchor links at the top; each must resolve to a real
        // section id so the nav can never 404 the visitor within the page.
        const doc = parse();
        const nav = doc.querySelector("nav.nav-links");
        expect(nav, "header nav missing").toBeTruthy();
        const hrefs = [...nav!.querySelectorAll("a")].map((a) =>
            a.getAttribute("href"),
        );
        expect(hrefs).toEqual(["#features", "#roadmap"]);
        for (const href of hrefs) {
            const target = doc.getElementById(href!.slice(1));
            expect(target, `nav anchor ${href} resolves to nothing`).toBeTruthy();
        }
    });

    it("lists every shipped release up to the current one", () => {
        // The full release history in one short line per version — from the
        // first feature release to the current one. Exactly ONE entry may
        // be marked current, and it must be v1.7.0 (Storestation).
        const doc = parse();
        const roadmap = doc.querySelector("section.roadmap#roadmap");
        expect(roadmap, "roadmap section missing").toBeTruthy();
        const text = (roadmap?.textContent ?? "").toLowerCase();
        for (const version of [
            "v0.2.0",
            "v1.0.0",
            "v1.0.3",
            "v1.5.0",
            "v1.6.0",
            "v1.6.1",
            "v1.7.0",
        ]) {
            expect(text, `shipped version ${version} missing`).toContain(version);
        }
        expect(text).toMatch(/storestation/);

        const items = [...roadmap!.querySelectorAll(".timeline-item")];
        const current = items.filter((i) => i.classList.contains("now"));
        expect(current, "exactly one roadmap entry may be current").toHaveLength(1);
        expect(current[0]?.textContent ?? "").toMatch(/v1\.7\.0/);
    });

    it("lists every planned release without drowning in detail", () => {
        // The planned ladder, one short line each — v1.7.5 through v2.6.0
        // plus the v3.0.0 ecosystem finale (development branch). Each line
        // stays a headline: no dates, no scope essays.
        const doc = parse();
        const roadmap = doc.querySelector("section.roadmap#roadmap");
        expect(roadmap, "roadmap section missing").toBeTruthy();
        const text = (roadmap?.textContent ?? "").toLowerCase();
        for (const [version, topic] of [
            ["v1.7.5", "always-on"],
            ["v1.8.0", "live cli"],
            ["v1.9.0", "terminal"],
            ["v2.0.0", "agents view"],
            ["v2.1.0", "herdr"],
            ["v2.2.0", "command palette"],
            ["v2.3.0", "pinned tabs"],
            ["v2.4.0", "teammates"],
            ["v2.5.0", "ssh view"],
            ["v2.6.0", "multi-window"],
            ["v3.0.0", "ecosystem"],
        ] as const) {
            expect(text, `planned ${version} missing`).toContain(version);
            expect(text, `${version} must name its topic (${topic})`).toContain(topic);
        }
        // One short line per version — no timeline entry grows an essay.
        const items = [...roadmap!.querySelectorAll(".timeline-item .what")];
        for (const item of items) {
            expect(
                (item.textContent ?? "").trim().length,
                `timeline line too long: "${item.textContent}"`,
            ).toBeLessThan(140);
        }
    });

    it("resolves every internal anchor on the page", () => {
        // Every href="#name" (the whole page, not just the nav) must point
        // at an existing id — a dead in-page anchor is a silent 404.
        const doc = parse();
        const internal = [...doc.querySelectorAll("a[href^='#']")];
        expect(internal.length, "the page should have internal anchors").toBeGreaterThan(0);
        for (const a of internal) {
            const id = a.getAttribute("href")!.slice(1);
            expect(
                doc.getElementById(id),
                `internal anchor #${id} resolves to nothing`,
            ).toBeTruthy();
        }
    });

    it("links only to known-good external destinations", () => {
        // "No dead external links" as a whitelist: every http(s) href must
        // live on a host the project actually controls or explicitly relies
        // on. A new link to a typo'd or renamed host fails here before a
        // visitor hits it. Fetchability itself is the HITL click-through.
        const allowed = [
            "https://github.com/CrystalPlatforms/umux",
            "https://raw.githubusercontent.com/CrystalPlatforms/umux/",
            "https://umux.pages.dev/",
            "https://crystal-studio.dev",
            "https://img.shields.io/",
            "https://gc.zgo.at/",
        ];
        const doc = parse();
        const external = [
            ...doc.querySelectorAll("a[href^='http']"),
        ].map((a) => a.getAttribute("href")!);
        expect(external.length, "expected external links to audit").toBeGreaterThan(0);
        for (const href of external) {
            expect(
                allowed.some((base) => href.startsWith(base)),
                `unexpected external destination: ${href}`,
            ).toBe(true);
        }
    });

    it("never mentions telemetry in the roadmap either", () => {
        // Same standing rule as the base page (Adam, 2026-09-16), extended
        // over the roadmap: the topic stays off the page entirely.
        expect(html.toLowerCase()).not.toContain("telemetry");
    });
});
