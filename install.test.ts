// install.test.ts — smoke tests for install.sh (issue #65).
//
// Public interface under test: `sh install.sh` exactly as a user (or the
// curl|sh pipe) runs it. Everything is forced deterministic through the
// script's own environment overrides (UMUX_OS / UMUX_ARCH / UMUX_VERSION —
// the same hooks the issue's verification steps call for), so the dry-run
// tests never touch the network. The real download+extract path is verified
// by running the script for real (done manually on macOS against v1.0.4:
// dmg mount + deb extraction both produce a working `umux --version`).
//
// Assumptions encoded here:
//  - Tags are v-prefixed ("v1.0.4") but ASSET names embed the bare version
//    ("umux_1.0.4_universal.dmg") — the script must normalize between them.
//  - Windows is out of scope (no release assets) and must fail with a clear
//    message and a non-zero exit.
//  - Skipped on Windows hosts entirely: the script is POSIX sh; dev-testing
//    it there would need WSL, which is not part of this issue.

import { describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "install.sh");

const run = (extraEnv: Record<string, string>) =>
    spawnSync("sh", [script, "--dry-run"], {
        encoding: "utf8",
        env: { ...process.env, ...extraEnv },
    });

describe("install.sh smoke (issue #65)", () => {
    it.skipIf(process.platform === "win32")(
        "dry-run prints the detected platform, pinned version and chosen asset",
        () => {
            // Pinned version → fully deterministic, offline. macOS is forced
            // so the assertion is identical on any dev machine.
            const r = run({
                UMUX_OS: "macos",
                UMUX_ARCH: "aarch64",
                UMUX_VERSION: "1.0.4",
            });
            expect(r.status).toBe(0);
            expect(r.stdout).toContain("dry run");
            expect(r.stdout).toContain("macos");
            expect(r.stdout).toContain("aarch64");
            expect(r.stdout).toContain("umux_1.0.4_universal.dmg");
        },
    );

    it.skipIf(process.platform === "win32")(
        "normalizes a v-prefixed pinned version to the bare asset version",
        () => {
            // Tags are "v1.0.4"; assets embed "1.0.4". A user pinning either
            // form must land on the same asset name.
            const r = run({
                UMUX_OS: "macos",
                UMUX_ARCH: "aarch64",
                UMUX_VERSION: "v1.0.4",
            });
            expect(r.status).toBe(0);
            expect(r.stdout).toContain("umux_1.0.4_universal.dmg");
        },
    );

    it.skipIf(process.platform === "win32")(
        "chooses the amd64 .deb for linux/x86_64",
        () => {
            const r = run({
                UMUX_OS: "linux",
                UMUX_ARCH: "x86_64",
                UMUX_VERSION: "1.0.4",
            });
            expect(r.status).toBe(0);
            expect(r.stdout).toContain("umux_1.0.4_amd64.deb");
        },
    );

    it.skipIf(process.platform === "win32")(
        "fails with a clear message on an unsupported platform",
        () => {
            const r = run({ UMUX_OS: "windows", UMUX_VERSION: "1.0.4" });
            expect(r.status).not.toBe(0);
            const out = `${r.stdout}${r.stderr}`;
            expect(out).toMatch(/unsupported platform/i);
            expect(out).toMatch(/releases/);
        },
    );

    it.skipIf(process.platform === "win32")(
        "fails for linux/aarch64 while only x86_64 Linux builds exist",
        () => {
            const r = run({
                UMUX_OS: "linux",
                UMUX_ARCH: "aarch64",
                UMUX_VERSION: "1.0.4",
            });
            expect(r.status).not.toBe(0);
            expect(`${r.stdout}${r.stderr}`).toMatch(/x86_64 only|unsupported/i);
        },
    );

    // --- Issue #90 (v1.7.0 Phase 8): the installer carries the daemon too ---
    //
    // Since v1.7.0 the script installs TWO binaries (the `umux` CLI and the
    // `umux-storestation` daemon). The dry-run must announce both targets so
    // a cautious user previews the full footprint; the release-note path for
    // pre-v1.7.0 images (daemon absent → CLI-only install) is exercised by
    // running the script for real against a pinned old version (HITL).

    it.skipIf(process.platform === "win32")(
        "lists the umux-storestation daemon beside the CLI among the install targets",
        () => {
            for (const os of ["macos", "linux"]) {
                const r = run({
                    UMUX_OS: os,
                    UMUX_ARCH: os === "macos" ? "aarch64" : "x86_64",
                    UMUX_VERSION: "1.7.0",
                });
                expect(r.status, `${os} dry-run must succeed`).toBe(0);
                const targets = r.stdout.split("\n").filter((l) => l.includes("target:"));
                expect(
                    targets,
                    `${os} dry-run must print two install targets`,
                ).toHaveLength(2);
                expect(r.stdout).toMatch(new RegExp(`target:\\s+\\S+/umux$`, "m"));
                expect(r.stdout).toMatch(
                    new RegExp(`target:\\s+\\S+/umux-storestation$`, "m"),
                );
            }
        },
    );
});
