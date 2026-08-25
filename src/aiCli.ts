// aiCli — the known AI-CLI program names (agent-status presence, model v2,
// HITL 2026-08-25).
//
// The backend reports each panel's foreground process name — argv[0]'s
// basename, i.e. the word the user typed, NEVER terminal content — and this
// closed list decides which programs count as "an AI CLI is running here".
// Keeping it a deliberate, explicit list (not a guess) is the whole point;
// extend it as tools appear.

/// Known AI-CLI process names (lowercase basenames).
export const AI_CLI_PROCESS_NAMES = ['claude', 'codex', 'gemini', 'aider'] as const

/// Does this foreground process name belong to a known AI CLI?
/// Accepts a bare name or a path (basename compared), case-insensitively;
/// null/undefined/empty mean "no program" and never match.
export function isAiCliProcess(name: string | null | undefined): boolean {
  if (name == null || name === '') return false
  const base = name.split('/').pop() ?? name
  return (AI_CLI_PROCESS_NAMES as readonly string[]).includes(base.toLowerCase())
}
