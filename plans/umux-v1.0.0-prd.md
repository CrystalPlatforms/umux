# umux v1.0.0 — PRD (full cross-platform launch)

**Status:** shipped 2026-08-27 *(as release tags v1.0.1 / v1.0.2 — the v1.0.0 tag was taken by an earlier throwaway release; historical document)*
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.

## Problem Statement

After v0.2.0, umux existed only on Linux. Developers on Windows and macOS — the majority of the target audience — could not install it at all. The UI also lacked two glanceable pieces of context power users kept asking for: which git branch each terminal is on, and where a dev server is listening.

## Solution

v1.0.0 makes umux a **three-platform application**: GitHub Actions builds Windows (NSIS `.exe`) and macOS (universal `.dmg`) bundles alongside the Linux artifacts, with per-OS default shells and config directories and documented unsigned first-run workarounds. It adds **pane zoom** and **read-only sidebar metadata** (git branch on tab rows, listening ports in a hover tooltip), and puts a **landing page** on Cloudflare Pages — the gate for starting public promotion.

## User Stories

*(story numbers match the master PRD; all shipped)*

- **48.** As a developer, I want to zoom the focused panel to fill its tab and, with the same shortcut, return to the exact previous layout (other panels keep running untouched), so that I can read one agent's output closely without disturbing the rest.
- **49.** As a developer, I want each tab row in the sidebar to show the git branch of that tab's working directory (read-only display), so that I always know which branch each terminal is on at a glance.
- **50.** As a developer, I want each tab to surface the ports its shells are listening on (shown in a hover tooltip, not permanently), so that I can find a dev server without hunting through panels.

Platform scope (roadmap, not separate stories): Windows 10+ (NSIS) and macOS 11+ (universal binary: Apple Silicon + Intel) builds via GitHub Actions; per-OS shells and config directories (`%APPDATA%\umux`, `~/Library/Application Support/umux`); unsigned-first-run documentation (Gatekeeper/SmartScreen); landing page on Cloudflare Pages (`*.pages.dev`, zero cost); public promotion (Show HN, Reddit, X/Twitter, dev.to — English) starts only after the landing page is live.

## Implementation Decisions

- **CI matrix** — GitHub Actions, triggered on release publish, produces `.deb` + `.AppImage` (Linux), NSIS `-setup.exe` (Windows), and a universal `.dmg` (macOS).
- **Per-OS defaults** — default shell per OS (`$SHELL` or `/bin/bash` on Linux/macOS, PowerShell on Windows) and per-OS config directory resolution.
- **Zero-cost policy** — builds stay unsigned; first-run warnings are documented in the README, never paid away. Landing page on the free Cloudflare Pages tier; no paid hosting.
- **Pane zoom** — a layout-preserving toggle: zooming hides the siblings but does not tear down the split tree; un-zoom restores the exact previous geometry.
- **Sidebar metadata is read-only** — branch name and listening ports are display-only; git integration stays out of scope (clarified 2026-08-26).
- X11 sessions are untested and unsupported; Wayland is the Linux reference.

## Assumptions

- Unsigned builds are acceptable to early adopters when the workarounds are clearly documented.
- GitHub Actions free tiers and Cloudflare Pages free tier are sufficient for an open-source project's release cadence and landing traffic.
- Reading the git branch and listening ports of a tab's shells is reliable enough for read-only display.

## Tradeoffs Considered

- **Paid code-signing certificates** — rejected: zero-cost policy; documented workarounds instead.
- **Git integration (status, diffs, commits) in the sidebar** — rejected: read-only branch/ports display only; real git tooling stays out of scope.
- **X11 support** — rejected for now: untested; Wayland is the reference session.
- **A marketing site with paid hosting** — rejected: static landing page on Cloudflare Pages is free and fast enough.

## Validation Strategy

- **Zoom (story #48):** zoom the focused panel → it fills the tab; same shortcut → the exact previous layout returns with other panels still running.
- **Metadata (stories #49–#50):** tab rows show the current branch; hovering a tab row lists its listening ports; neither mutates anything.
- **Cross-platform acceptance:** Adam installs from CI artifacts on his Windows machine and his Mac — per-OS shell and config directory work, notifications fire.
- **CI:** each release publish attaches `.exe`, `.dmg`, and Linux artifacts.
- **Landing page:** live on Cloudflare Pages; promotion starts only after it is up.

## Out of Scope

- The `umux` CLI, import, and in-app updates (v1.2.0); umux Terminal (v1.3.0); native menus (v1.4.0).

## Further Notes

- Promotion rule introduced with this version: marketing content (produced with content skills, Adam ≤1 h/week; YouTube deferred) starts only after the landing page is live.
- Cross-platform ship history: v1.0.1 (2026-08-27), then v1.0.2 with confirmed asset names (same day).
