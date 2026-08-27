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
//    are bumped on each release (names confirmed with CI for v1.0.2 — see
//    removed-marketing-drafts). Windows ships NSIS only (.exe, no .msi).
//  - Badges are dynamic shields.io endpoints (they update themselves); we
//    assert the endpoint shape, not the current numbers.
//  - Media (demo GIF, screenshots) live under landing/assets/ and ship with
//    an inline-SVG onerror fallback until real files are dropped in.
//  - GoatCounter ships as a commented-out snippet (needs Adam's account
//    code); the page itself loads zero external scripts.
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
        // Real assets copied from public-assets/ into landing/assets/ (Pages
        // root is landing/). The "❯" prompt glyph in front of the brand is
        // replaced by the logo image.
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
            "umux_1.0.2_amd64.AppImage",
            "umux_1.0.2_amd64.deb",
            "umux-1.0.2-1.x86_64.rpm",
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
        // CI actually produces (confirmed for v1.0.2 in removed-marketing-drafts);
        // on a new release only the version inside these names is bumped.
        const doc = parse();
        const hrefs = new Set(
            [...doc.querySelectorAll("a")].map((a) => a.getAttribute("href")),
        );
        const base = "https://github.com/CrystalPlatforms/umux/releases/latest/download";
        const assets = [
            "umux_1.0.2_amd64.AppImage", // Linux
            "umux_1.0.2_amd64.deb", // Linux
            "umux-1.0.2-1.x86_64.rpm", // Linux
            "umux_1.0.2_universal.dmg", // macOS (universal)
            "umux_1.0.2_x64-setup.exe", // Windows (NSIS only — no .msi)
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

    it("embeds the demo GIF and screenshots with a graceful placeholder", () => {
        // Media live under landing/assets/ with fixed names so Adam only ever
        // copies files in — no HTML edits. Until then, every media element
        // must degrade to an inline SVG placeholder, never a broken image.
        const doc = parse();
        const media = [...doc.querySelectorAll("img")].filter((img) =>
            /assets\/(demo\.gif|screenshot-.+\.png)$/.test(img.getAttribute("src") ?? ""),
        );
        const srcs = media.map((img) => img.getAttribute("src"));
        expect(srcs, "hero demo GIF missing").toContain("assets/demo.gif");
        for (const platform of ["linux", "macos", "windows"]) {
            expect(
                srcs,
                `screenshot for ${platform} missing`,
            ).toContain(`assets/screenshot-${platform}.png`);
        }
        for (const img of media) {
            expect(
                img.getAttribute("onerror"),
                `${img.getAttribute("src")} must fall back to the inline placeholder`,
            ).toContain("data:image/svg+xml");
        }
    });

    it("leads with the one-line pitch and the three v1 feature highlights", () => {
        // Issue #35 content list: hero pitch + exactly three highlights —
        // workspaces & panels, agent status & notifications, session restore.
        const doc = parse();
        expect(doc.querySelector("h1")?.textContent ?? "").toMatch(/terminal workspace/i);

        const features = [...doc.querySelectorAll(".feature")];
        expect(features, "expected exactly three feature highlights").toHaveLength(3);
        const text = features
            .map((f) => f.textContent?.toLowerCase() ?? "")
            .join("\n");
        expect(text).toMatch(/workspace/);
        expect(text).toMatch(/panel/);
        expect(text).toMatch(/agent status/);
        expect(text).toMatch(/notification/);
        expect(text).toMatch(/session restore/);
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

    it("loads zero external scripts; GoatCounter ships opt-in", () => {
        // "Loads fast" AC: nothing is fetched from the network to run the
        // page — no script may carry a src (the dropdown/dialog UI logic is
        // one small inline script). Stats stay cookie-free AND off by
        // default: the GoatCounter snippet ships commented out and is
        // activated only once Adam's account code is filled in.
        const doc = parse();
        expect(doc.querySelectorAll("script[src]")).toHaveLength(0);
        expect(html, "commented-out GoatCounter snippet missing").toMatch(
            /<!--[\s\S]*gc\.zgo\.at[\s\S]*-->/,
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
            ["AppImage", /chmod \+x/, "umux_1.0.2_amd64.AppImage"],
            [".deb", /apt install/, "umux_1.0.2_amd64.deb"],
            [".rpm", /dnf install/, "umux-1.0.2-1.x86_64.rpm"],
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
});
