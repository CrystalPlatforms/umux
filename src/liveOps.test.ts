// liveOps — the inherently-live op catalog's behavior (#127 I/O, #128
// system actions). The deps are fakes, so every op is driven exactly the
// way the gateway drives it: op + params in, the `gateway_op_respond`
// payload out. The suite pins the #128 system actions' contracts:
//
//  - update.check mirrors the Settings row's settled states (the SAME
//    runCheck machine — preflight settles before the plugin is asked);
//  - update.install answers FIRST (installed + relaunchScheduled) and only
//    relaunches after the grace window — the answer describes what happens,
//    the app performs it;
//  - reset.run is the Settings arm's semantics (reset_all + relaunch);
//  - open.folder falls back to reveal exactly like the double-click;
//  - open.port builds the http://localhost URL (no clipboard copy);
//  - window.* answer before the window action lands;
//  - tabs.ports / tabs.branches reuse the tooltip/label queries verbatim;
//  - tabs.fullscreen shares the tab menu's transient mode with its
//    preconditions (tab exists, workspace open);
//  - an unknown op returns false — the layout machine's signal.
//
// NOT tested here: the gateway transport (storestation wire suites), the
// updater plugin round-trip (updater.ts tests), the real window API.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createWorkspace,
  addTab,
  emptyState,
  openWorkspace,
  type WorkspaceState,
} from './workspaces'
import { handleLiveOp, type LiveOpDeps, type PtyEntry } from './liveOps'
import { defaultSettings } from './settings'
import type { UpdateResource } from './updater'

const seq = (): (() => string) => {
  let n = 0
  return () => `g${(n += 1)}`
}

function seeded(): WorkspaceState {
  const gen = seq()
  let s = emptyState
  s = createWorkspace(s, 'projekt', gen)
  s = addTab(s, s.workspaces[0].id, gen)
  s = openWorkspace(s, s.workspaces[0].id)
  return s
}

const firstTabId = (s: WorkspaceState): string => s.workspaces[0].tabs![0].id
const secondTabId = (s: WorkspaceState): string => s.workspaces[0].tabs![1].id

type Recorded = { cmd: string; args?: Record<string, unknown> }

function harness(state: WorkspaceState = seeded(), shells: Record<string, PtyEntry> = {}) {
  const calls: Recorded[] = []
  // ONE window spy triple — deps.window() must return the SAME spies every
  // time, or the assertions would look at a fresh object.
  const winSpies = {
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  }
  const deps: LiveOpDeps = Object.assign(
    {
      invoke: Object.assign(async <T,>(): Promise<T> => undefined as unknown as T, {
        // Record gateway answers and command invokes alike.
      }) as LiveOpDeps['invoke'],
      state: () => state,
      settings: () => defaultSettings,
      liveShell: (panelId: string | null): PtyEntry | null =>
        panelId != null ? shells[panelId] ?? null : null,
      applySettingPatch: () => {},
      relaunch: vi.fn(),
      window: () => winSpies,
      openPath: vi.fn(async () => {}),
      revealItemInDir: vi.fn(async () => {}),
      openUrl: vi.fn(async () => {}),
      updaterApi: {
        preflight: vi.fn(async () => 'ok' as const),
        check: vi.fn(async () => null),
        install: vi.fn(async () => {}),
        relaunch: vi.fn(),
      },
      setFullscreen: vi.fn(),
    } satisfies LiveOpDeps,
    {},
  )
  // Wrap the invoke to RECORD after delegation (the default no-op resolve).
  const rawInvoke = deps.invoke
  deps.invoke = (async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ cmd, args })
    return rawInvoke<T>(cmd, args)
  }) as LiveOpDeps['invoke']
  const answers: Array<Record<string, unknown>> = []
  const original = deps.invoke
  deps.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'gateway_op_respond' && args != null) {
      answers.push(args['payload'] as Record<string, unknown>)
      return undefined as unknown
    }
    return (original as LiveOpDeps['invoke'])(cmd, args)
  }) as LiveOpDeps['invoke']
  return { deps, calls, answers }
}

const idsOf = (s: WorkspaceState): string[] => {
  const first = s.workspaces[0].tabs![0]
  return [firstTabId(s), secondTabId(s), first.id].slice(0, 2)
}

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
})

/// Flush the microtask chain an op's .then lives on.
const settle = () => vi.advanceTimersByTimeAsync(0)

describe('liveOps — update.check (#128)', () => {
  it('mirrors the Settings row: up-to-date when the check answers null', async () => {
    const { deps, answers } = harness()
    deps.updaterApi.check = vi.fn(async () => null)
    expect(handleLiveOp(deps, 1, 'update.check', {})).toBe(true)
    await settle()
    expect(answers).toEqual([
      { ok: true, result: { update: { state: 'up-to-date' } } },
    ])
  })

  it('reports an available version the way the row does', async () => {
    const { deps, answers } = harness()
    deps.updaterApi.check = vi.fn(async () => ({ version: '9.9.0' }) as UpdateResource)
    handleLiveOp(deps, 1, 'update.check', {})
    await settle()
    expect(answers).toEqual([
      { ok: true, result: { update: { state: 'available', version: '9.9.0' } } },
    ])
  })

  it('pre-flight settles quietly: no-release is a RESULT, not an error', async () => {
    const { deps, answers } = harness()
    deps.updaterApi.preflight = vi.fn(async () => 'no-release' as const)
    handleLiveOp(deps, 1, 'update.check', {})
    await settle()
    expect(answers).toEqual([
      { ok: true, result: { update: { state: 'no-release' } } },
    ])
  })

  it('offline and other error states answer the enumerated error', async () => {
    const { deps, answers } = harness()
    deps.updaterApi.preflight = vi.fn(async () => 'offline' as const)
    handleLiveOp(deps, 1, 'update.check', {})
    await settle()
    expect(answers[0]['ok']).toBe(false)
    const error = answers[0]['error'] as { code: string; message: string }
    expect(error.code).toBe('ioError')
    expect(error.message).toMatch(/offline/i)
  })
})

describe('liveOps — update.install (#128)', () => {
  it('installs, answers the relaunch boundary, then relaunches', async () => {
    const { deps, answers } = harness()
    const update = { version: '9.9.0' } as UpdateResource
    deps.updaterApi.check = vi.fn(async () => update)
    const install = vi.fn(async () => {})
    deps.updaterApi.install = install
    handleLiveOp(deps, 1, 'update.install', {})
    await settle()
    // The answer describes what WILL happen…
    expect(answers).toEqual([
      {
        ok: true,
        result: { installed: true, version: '9.9.0', relaunchScheduled: true },
      },
    ])
    expect(install).toHaveBeenCalledWith(update, expect.any(Function))
    expect(deps.relaunch).not.toHaveBeenCalled()
    // …and the app performs it after the grace window.
    await vi.advanceTimersByTimeAsync(800)
    expect(deps.relaunch).toHaveBeenCalledTimes(1)
  })

  it('installs nothing when the check says up-to-date', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'update.install', {})
    await settle()
    expect(answers).toEqual([
      {
        ok: true,
        result: { installed: false, update: { state: 'up-to-date' } },
      },
    ])
    expect(deps.updaterApi.install).not.toHaveBeenCalled()
    expect(deps.relaunch).not.toHaveBeenCalled()
  })
})

describe('liveOps — reset.run (#128)', () => {
  it('wipes via reset_all and relaunches after answering', async () => {
    const { deps, calls, answers } = harness()
    handleLiveOp(deps, 1, 'reset.run', {})
    await settle()
    expect(calls.some((c) => c.cmd === 'reset_all')).toBe(true)
    expect(answers).toEqual([
      { ok: true, result: { reset: true, relaunchScheduled: true } },
    ])
    expect(deps.relaunch).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(800)
    expect(deps.relaunch).toHaveBeenCalledTimes(1)
  })

  it('answers the ioError when the wipe fails', async () => {
    const { deps, answers } = harness()
    const original = deps.invoke
    deps.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'reset_all') throw new Error('disk gone')
      return original(cmd, args)
    }) as LiveOpDeps['invoke']
    handleLiveOp(deps, 1, 'reset.run', {})
    await settle()
    expect(answers[0]['ok']).toBe(false)
    expect((answers[0]['error'] as { code: string }).code).toBe('ioError')
    expect(deps.relaunch).not.toHaveBeenCalled()
  })
})

describe('liveOps — open.folder / open.port (#128)', () => {
  it('opens the folder; a refused contents view falls back to reveal', async () => {
    const { deps, answers } = harness()
    deps.openPath = vi.fn(async () => {})
    handleLiveOp(deps, 1, 'open.folder', { target: '/home/me/projekt' })
    await settle()
    expect(deps.openPath).toHaveBeenCalledWith('/home/me/projekt')
    expect(deps.revealItemInDir).not.toHaveBeenCalled()
    expect(answers).toEqual([
      { ok: true, result: { opened: '/home/me/projekt' } },
    ])
  })

  it('reveals the folder when opening the contents refuses', async () => {
    const { deps, answers } = harness()
    deps.openPath = vi.fn(async () => {
      throw new Error('no handler')
    })
    deps.revealItemInDir = vi.fn(async () => {})
    handleLiveOp(deps, 1, 'open.folder', { target: '/gone' })
    await settle()
    expect(deps.revealItemInDir).toHaveBeenCalledWith('/gone')
    expect(answers).toEqual([{ ok: true, result: { opened: '/gone' } }])
  })

  it('opens the http://localhost URL for a port click', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'open.port', { tab: 't1', port: 5173 })
    await settle()
    expect(deps.openUrl).toHaveBeenCalledWith('http://localhost:5173')
    expect(answers).toEqual([
      { ok: true, result: { opened: 'http://localhost:5173', tab: 't1' } },
    ])
  })

  it('refuses a malformed port with badParams', async () => {
    for (const port of [0, 65536, '5173', null]) {
      const { deps, answers } = harness()
      handleLiveOp(deps, 1, 'open.port', { tab: 't1', port })
      await settle()
      expect((answers[0]['error'] as { code: string }).code).toBe('badParams')
      expect(deps.openUrl).not.toHaveBeenCalled()
    }
  })
})

describe('liveOps — window actions (#128)', () => {
  it('minimizes and answers', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'window.minimize', {})
    await settle()
    expect(deps.window()!.minimize).toHaveBeenCalledTimes(1)
    expect(answers).toEqual([{ ok: true, result: { minimized: true } }])
  })

  it('toggles maximize', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'window.toggle-maximize', {})
    await settle()
    expect(deps.window()!.toggleMaximize).toHaveBeenCalledTimes(1)
    expect(answers).toEqual([{ ok: true, result: { toggled: true } }])
  })

  it('answers BEFORE the window closes (the grace window)', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'window.close', {})
    await settle()
    expect(answers).toEqual([{ ok: true, result: { closing: true } }])
    expect(deps.window()!.close).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(800)
    expect(deps.window()!.close).toHaveBeenCalledTimes(1)
  })
})

describe('liveOps — tabs.ports / tabs.branches (#128)', () => {
  it('queries the tab\'s LOCAL panel handles like the tooltip does', async () => {
    const s = seeded()
    const id = firstTabId(s)
    const shells: Record<string, PtyEntry> = {
      'leaf-1': { kind: 'local', id: 7 },
    }
    // The tab's layout leaf ids come from PaneLayout; make the fake shell
    // match the seeded layout's first leaf.
    const leafId = leafIdOf(s, id, 0)
    shells[leafId] = { kind: 'local', id: 7 }
    const { deps, calls, answers } = harness(s, shells)
    handleLiveOp(deps, 1, 'tabs.ports', { tab: id })
    await settle()
    const query = calls.find((c) => c.cmd === 'tab_ports')
    expect(query?.args).toEqual({ tabs: [{ tabId: id, ptyIds: [7] }] })
    expect(answers[0]['ok']).toBe(true)
  })

  it('answers an explicit empty list without a round-trip when no local shell is up', async () => {
    const s = seeded()
    const { deps, calls, answers } = harness(s, {})
    handleLiveOp(deps, 1, 'tabs.ports', { tab: firstTabId(s) })
    await settle()
    expect(calls.find((c) => c.cmd === 'tab_ports')).toBeUndefined()
    expect(answers).toEqual([
      { ok: true, result: { tab: firstTabId(s), ports: [] } },
    ])
  })

  it('answers the per-panel labels with the focused flag', async () => {
    const s = seeded()
    const id = firstTabId(s)
    const leaf = leafIdOf(s, id, 0)
    const { deps, calls, answers } = harness(s, {
      [leaf]: { kind: 'local', id: 1 },
    })
    // The seeded panel carries no workingDirectory → no dir → no query;
    // answer a dir-carrying state instead: patch the panel's cwd.
    const patched = {
      ...s,
      workspaces: s.workspaces.map((ws) => ({
        ...ws,
        panels: [
          { id: leaf, workingDirectory: '/repo', sshTarget: null },
        ],
      })),
    } as WorkspaceState
    const h2 = harness(patched, { [leaf]: { kind: 'local', id: 1 } })
    handleLiveOp(h2.deps, 1, 'tabs.branches', { tab: id })
    await settle()
    const query = h2.calls.find((c) => c.cmd === 'git_branches')
    expect(query?.args).toEqual({ dirs: ['/repo'] })
    expect(h2.answers[0]['ok']).toBe(true)
    void deps
    void calls
    void answers
  })

  it('session restore OFF surfaces no dirs (the labels vanish with it)', async () => {
    const s = seeded()
    const leaf = leafIdOf(s, firstTabId(s), 0)
    const patched = {
      ...s,
      workspaces: s.workspaces.map((ws) => ({
        ...ws,
        panels: [{ id: leaf, workingDirectory: '/repo', sshTarget: null }],
      })),
    } as WorkspaceState
    const { deps, calls, answers } = harness(patched, {
      [leaf]: { kind: 'local', id: 1 },
    })
    deps.settings = () => ({ ...defaultSettings, sessionRestoreEnabled: false })
    handleLiveOp(deps, 1, 'tabs.branches', { tab: firstTabId(patched) })
    await settle()
    expect(calls.find((c) => c.cmd === 'git_branches')).toBeUndefined()
    const result = answers[0]['result'] as {
      panels: Array<{ branch: string | null }>
    }
    expect(result.panels.every((p) => p.branch === null)).toBe(true)
  })

  it('an unknown tab answers tabNotFound', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'tabs.ports', { tab: 'nope' })
    await settle()
    expect((answers[0]['error'] as { code: string }).code).toBe('tabNotFound')
  })
})

describe('liveOps — tabs.fullscreen (#128)', () => {
  it('enters the mode on an open workspace\'s tab via the SAME setter', async () => {
    const s = seeded()
    const { deps, answers } = harness(s)
    handleLiveOp(deps, 1, 'tabs.fullscreen', { tab: firstTabId(s), on: true })
    await settle()
    expect(deps.setFullscreen).toHaveBeenCalledWith(firstTabId(s))
    expect(answers).toEqual([
      { ok: true, result: { fullscreen: { tab: firstTabId(s), on: true } } },
    ])
  })

  it('leaves the mode (the ESC path) with on:false', async () => {
    const s = seeded()
    const { deps, answers } = harness(s)
    handleLiveOp(deps, 1, 'tabs.fullscreen', { tab: firstTabId(s), on: false })
    await settle()
    expect(deps.setFullscreen).toHaveBeenCalledWith(null)
    expect(answers[0]['result']).toEqual({
      fullscreen: { tab: firstTabId(s), on: false },
    })
  })

  it('refuses to enter on a closed workspace (no surface to hide into)', async () => {
    const s = seeded()
    const closed = { ...s, openIds: [] }
    const { deps, answers } = harness(closed)
    handleLiveOp(deps, 1, 'tabs.fullscreen', { tab: firstTabId(s), on: true })
    await settle()
    expect((answers[0]['error'] as { code: string }).code).toBe(
      'workspaceNotFound',
    )
    expect(deps.setFullscreen).not.toHaveBeenCalled()
  })

  it('refuses an unknown tab and malformed params', async () => {
    const { deps, answers } = harness()
    handleLiveOp(deps, 1, 'tabs.fullscreen', { tab: 'nope', on: true })
    await settle()
    expect((answers[0]['error'] as { code: string }).code).toBe('tabNotFound')
    const h2 = harness()
    handleLiveOp(h2.deps, 1, 'tabs.fullscreen', { tab: 't1' })
    await settle()
    expect((h2.answers[0]['error'] as { code: string }).code).toBe('badParams')
  })
})

describe('liveOps — the catalog boundary', () => {
  it('an unknown op returns false — the layout machine\'s signal', () => {
    const { deps } = harness()
    expect(handleLiveOp(deps, 1, 'not.an.op', {})).toBe(false)
    expect(handleLiveOp(deps, 1, 'workspaces.create', {})).toBe(false)
  })

  it('every LIVE_OPS entry is handled (the fixture suite holds the list)', () => {
    const { deps } = harness()
    for (const op of LIVE_OPS_LIST) {
      expect(handleLiveOp(deps, 1, op, {}), `"${op}" must be handled`).toBe(true)
    }
  })
})

import { LIVE_OPS as LIVE_OPS_LIST } from './liveOps'

/// The seeded tab layout's i-th leaf id (PaneLayout leaf ids are the panel
/// ids; the seeded one-tab layout has exactly one leaf).
function leafIdOf(s: WorkspaceState, _tabId: string, _i: number): string {
  const tab = s.workspaces[0].tabs![0]
  const layout = tab.layout as { id: string } | { first: unknown; second: unknown; id: string }
  if ('first' in layout) throw new Error('the seeded fixture has a single-leaf layout')
  return layout.id
}
