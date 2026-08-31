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
//    `releases/latest/download/<asset>`; asset names embed the version and
//    are bumped on each release (names confirmed with CI for v1.0.2).
//    Windows ships NSIS only (.exe, no .msi).
//  - Badges are dynamic shields.io endpoints (they update themselves); we
//    assert the endpoint shape, not the current numbers.
//  - Media (demo GIF, screenshots) live under landing/assets/ and ship with
//    an inline-SVG onerror fallback until real files are dropped in.
//  - GoatCounter is ACTIVE (account `crystalstudio`, 2026-08-27) and is the
//    page's single external script; the UI logic itself stays inline.
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
            "umux_1.0.4_amd64.AppImage",
            "umux_1.0.4_amd64.deb",
            "umux-1.0.4-1.x86_64.rpm",
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
        // which always resolves to the newest release. Asset names are the ones
        // CI actually produces (confirmed for v1.0.2);
        // on a new release only the version inside these names is bumped.
        const doc = parse();
        const hrefs = new Set(
            [...doc.querySelectorAll("a")].map((a) => a.getAttribute("href")),
        );
        const base = "https://github.com/CrystalPlatforms/umux/releases/latest/download";
        const assets = [
            "umux_1.0.4_amd64.AppImage", // Linux
            "umux_1.0.4_amd64.deb", // Linux
            "umux-1.0.4-1.x86_64.rpm", // Linux
            "umux_1.0.4_universal.dmg", // macOS (universal)
            "umux_1.0.4_x64-setup.exe", // Windows (NSIS only — no .msi)
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
        // Adam's shots (2026-08-27): umux-first is the big hero image,
        // umux-agent illustrates the agent-status feature, umux-session the
        // session-restore one. All live in landing/assets/. A broken image
        // must still degrade to the inline placeholder, never a broken-icon.
        const doc = parse();
        const hero = doc.querySelector(".demo-frame img");
        expect(hero?.getAttribute("src")).toBe("assets/umux-first.png");

        const features = [...doc.querySelectorAll(".feature")];
        const byHeading = (re: RegExp) =>
            features.find((f) => re.test(f.querySelector("h2")?.textContent ?? ""));
        expect(
            byHeading(/agent status/i)?.querySelector("img")?.getAttribute("src"),
        ).toBe("assets/umux-agent.png");
        expect(
            byHeading(/session restore/i)?.querySelector("img")?.getAttribute("src"),
        ).toBe("assets/umux-session.png");

        const media = [
            hero,
            byHeading(/agent status/i)?.querySelector("img"),
            byHeading(/session restore/i)?.querySelector("img"),
        ];
        for (const img of media) {
            expect(
                img?.getAttribute("onerror"),
                `${img?.getAttribute("src")} must fall back to the inline placeholder`,
            ).toContain("data:image/svg+xml");
        }
        // The per-OS screenshot strip is gone — no stale references left.
        expect(html).not.toMatch(/demo\.gif|screenshot-(linux|macos|windows)\.png/);
    });

    it("leads with the one-line pitch and the six feature tiles", () => {
        // Hero pitch + six tiles: the three v1 highlights (workspaces & panels,
        // agent status & notifications, session restore) plus import from cmux
        // (shipped), import from herdr and embedded terminal (both announced as
        // not-yet-available).
        const doc = parse();
        expect(doc.querySelector("h1")?.textContent ?? "").toMatch(/terminal workspace/i);

        const features = [...doc.querySelectorAll(".feature")];
        expect(features, "expected six feature tiles").toHaveLength(6);
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
        // Unbuilt features must say so up front on their tiles.
        const byHeading = (re: RegExp) =>
            features.find((f) => re.test(f.querySelector("h2")?.textContent ?? ""));
        for (const tile of [byHeading(/herdr/i), byHeading(/embedded terminal/i)]) {
            expect(tile?.textContent ?? "").toMatch(/coming soon/i);
        }
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
            ["AppImage", /chmod \+x/, "umux_1.0.4_amd64.AppImage"],
            [".deb", /apt install/, "umux_1.0.4_amd64.deb"],
            [".rpm", /dnf install/, "umux-1.0.4-1.x86_64.rpm"],
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
