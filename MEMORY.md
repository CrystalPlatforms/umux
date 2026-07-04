MEMORY

#CRYSTAL 

Purpose & context
Adam is building umux, an open-source Ubuntu terminal workspace manager — a simplified Linux equivalent of the macOS tool cmux. The core problem it solves: scattered, disorganized terminal windows, addressed by grouping them into per-project workspaces. The project is public on GitHub, has no hard deadline, and success is validated by Adam testing locally.
Tech stack (Claude-recommended, Adam confirmed): Tauri v2 + React + TypeScript (frontend), Rust (backend).
Key scoping decisions made:
* In scope: Window manager with workspaces, split terminal layout (max 2 panels per workspace), drag-and-drop resize, SSH support, system notifications triggered when AI CLI tools (Claude Code, Aider, etc.) finish generating
* Out of scope: Browser integration, Git integration
* Notification mechanism: umux intercepts OSC 9/99/777 escape sequences from the PTY stream (emitted automatically by Claude Code) and triggers Ubuntu system notifications via libnotify — no process monitoring or pattern matching needed
Current state
Two planning documents have been generated and saved to ./plans/:
* umux-prd.md — product requirements document
* umux-plan.md — implementation plan (19 granular atomic phases)
The 19 phases cover: project scaffolding → PTY backend → Tauri command bridge → terminal renderer → basic window layout → workspace data model → workspace UI → workspace/PTY isolation → split layout → drag-and-drop resize → OSC sequence parser → system notifications → SSH manager (HITL) → SSH UI → state persistence → error handling → UI polish (HITL) → performance/cleanup → README/release prep.
Both documents are in American English. The immediate next step is running /dispatch to convert the plan into GitHub issues.
Approach & patterns
* Adam uses a structured custom skill workflow: /ask (discovery) → /blueprint (PRD) → /carve (implementation planning) → /dispatch (GitHub issues)
* Prefers granular, atomic phase breakdowns (expanded from 7 to 19 phases on request)
* Communicates in Polish; final deliverables translated to American English
* Planning artifacts are written to a ./plans/ directory
