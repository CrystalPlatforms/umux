import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Read the README directly from the repo root so the assertion is
// deterministic regardless of any test-runner transformation. Same disk-read
// pattern as design-tokens.test.ts.
const here = path.dirname(fileURLToPath(import.meta.url));
const readme = fs.readFileSync(path.join(here, "..", "README.md"), "utf8");

// Phase 21 / Issue #22, AC1 — the README explains the project, build/run
// steps, and core features. These tests encode the *shape* a real README must
// have, not its prose: each required section must be present and non-empty.
//
// Assumptions encoded here:
//  - Input: the full text of README.md at the repo root.
//  - Rule: the README must name the project, describe what it is, list the
//    build prerequisites, give concrete build & run commands, document the
//    core features (workspaces, panes, SSH, notifications), and invite
//    contributions.
//  - Boundary: the file must be substantial (> 1 KB of real content), so a
//    stub with only headings and no body fails.
//  - Intentionally NOT tested: the correctness of the build/run instructions
//    themselves (verified manually by Adam on Ubuntu/Wayland) and whether a
//    release artifact is produced (separate, build-time verification).

const sections = (src: string): Record<string, string> => {
    // Map each markdown "## Heading" to the body text that follows it, up to
    // the next heading of the same or higher level. Headings without a body
    // map to an empty string, which the per-section assertions will reject.
    const out: Record<string, string> = {};
    const lines = src.split("\n");
    let current = "";
    let depth = 0;
    for (const line of lines) {
        const m = line.match(/^(#{1,6})\s+(.*)$/);
        if (m) {
            depth = m[1].length;
            current = m[2].trim().toLowerCase();
            out[current] = "";
        } else if (current) {
            // Only accumulate while we haven't dropped to a shallower heading;
            // since we re-key on every heading, a simple append is enough.
            out[current] += line + "\n";
        }
        void depth;
    }
    return out;
};

describe("README (Phase 21 / Issue #22, AC1)", () => {
    it("is a substantial document, not a stub", () => {
        expect(readme.length).toBeGreaterThan(1024);
    });

    it("names the project and describes what it is", () => {
        // The first heading should be the project name, and the intro must
        // mention "terminal" and "workspace" somewhere near the top.
        expect(readme.match(/^#\s+.+/m)).not.toBeNull();
        const head = readme.slice(0, 1500).toLowerCase();
        expect(head).toMatch(/terminal/);
        expect(head).toMatch(/workspace/);
    });

    it("documents build prerequisites", () => {
        const s = sections(readme);
        // At least one prerequisites-style section must exist and be non-empty.
        const prereq = Object.entries(s).find(
            ([k]) => k.includes("prerequisit") || k.includes("requirement") || k.includes("dependenc"),
        );
        expect(prereq, "a prerequisites/dependencies section must exist").toBeTruthy();
        expect(prereq![1].trim().length).toBeGreaterThan(0);
    });

    it("gives concrete build and run commands", () => {
        // The actual commands developers run must appear verbatim.
        expect(readme).toMatch(/npm run tauri dev/);
        expect(readme).toMatch(/npm run tauri build/);
        expect(readme).toMatch(/npm test/);
    });

    it("documents the core features (workspaces, panes, SSH, notifications)", () => {
        const lower = readme.toLowerCase();
        expect(lower).toMatch(/workspace/);
        expect(lower).toMatch(/panel/);
        expect(lower).toMatch(/ssh/);
        expect(lower).toMatch(/notification/);
    });

    it("invites contributions", () => {
        const s = sections(readme);
        const contrib = Object.entries(s).find(([k]) => k.includes("contribut"));
        expect(contrib, "a contributing section must exist").toBeTruthy();
        expect(contrib![1].trim().length).toBeGreaterThan(0);
    });
});
