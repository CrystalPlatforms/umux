// aiCli — unit tests (agent-status presence, model v2 / HITL 2026-08-25).
//
// The matcher is the whole module: a foreground process NAME (argv[0]
// basename from the backend) against the closed known-CLI list. Content of
// the terminal is never involved.

import { describe, it, expect } from 'vitest'
import { AI_CLI_PROCESS_NAMES, isAiCliProcess } from './aiCli'

describe('isAiCliProcess', () => {
  // T1 (every known CLI matches — bare name, full path, mixed case):
  it('matches every known CLI name in bare, path, and mixed-case form', () => {
    for (const name of AI_CLI_PROCESS_NAMES) {
      expect(isAiCliProcess(name)).toBe(true)
      expect(isAiCliProcess(`/usr/local/bin/${name}`)).toBe(true)
      expect(isAiCliProcess(name.toUpperCase())).toBe(true)
    }
  })

  // T2 (nothing else matches — shells, editors, empty, null):
  it('never matches unknown programs or empty input', () => {
    for (const junk of ['zsh', 'bash', 'vim', 'node', 'ssh', 'sleep', '', null, undefined]) {
      expect(isAiCliProcess(junk as string | null)).toBe(false)
    }
  })

  // T3 (a path only matches on its basename, not a directory component):
  it('compares the basename only', () => {
    expect(isAiCliProcess('/home/adam/claude-projects/nvim')).toBe(false)
    expect(isAiCliProcess('/opt/homebrew/bin/claude')).toBe(true)
  })
})
