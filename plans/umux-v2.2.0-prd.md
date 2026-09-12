# umux v2.2.0 — PRD (command palette + shortcut editor)

**Status:** planned — was v1.8.0 scope ("agent UX & convenience"); **renumbered 2026-09-12** into its own release.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.

## Problem Statement

Power users must memorize every shortcut because there is no way to browse all actions in one place, and defaults that feel awkward can only be endured — there is no GUI to rebind them.

## Solution

A **command palette** (Cmd/Ctrl+Shift+P) lists every app action — searchable, executable. A **GUI shortcut editor** in Settings shows all bindings and lets the user rebind any of them. Both consume one new foundation: the **ActionRegistry** — every app action listed exactly once (id, label, handler, binding). The v2.3.0 native menus render from the same registry, so palette, editor, and menus can never disagree.

## User Stories

*(story numbers match the master PRD)*

- **58.** As a user, I want a command palette (Cmd/Ctrl+Shift+P) listing all app actions, so that I can reach any function without memorizing shortcuts.
- **59.** As a user, I want to view and rebind keyboard shortcuts in Settings, so that I can replace defaults I find awkward — without editing config files.

## Implementation Decisions

- **ActionRegistry** *(new foundation, lands with this release)* — a single registry of every app action (id, label, handler, current binding). The palette and the shortcut editor are its first two consumers; **AppMenus (v2.3.0) is the third**, which is what structurally enforces the "every feature ships with its menu entry" rule (story #84).
- **Rebindings persist** in settings and survive restarts; conflicts show a clear warning.
- New features keep registering into the registry as they ship (standing expectation from v2.2.0 on).

## Assumptions

- Every app action can be expressed in the registry form (id, label, handler, binding) — including future features.

## Tradeoffs Considered

- **Palette without a shortcut editor** — rejected: the PO wants both reachability (palette) and personalization (rebinding); they share the registry anyway.
- **Introducing the registry together with menus (v2.3.0) instead** — rejected 2026-09-12: the palette needs the registry already; menus then reuse it one release later.

## Validation Strategy

- **Palette (#58):** the palette opens, lists every action, and executes them.
- **Editor (#59):** Adam rebinds a default shortcut in Settings and the new binding works after restart.
- **Registry completeness:** a test/check verifies every registered action appears in both the palette and the editor.

## Out of Scope

- Native menus (the registry's third consumer) — v2.3.0.
- Macro/command scripting beyond the CLI/socket surface — ecosystem/plugin territory.
- Custom theming of the palette beyond the app's default look — out of scope per the master PRD.
