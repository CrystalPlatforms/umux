// liveOps — the gateway's inherently-live op catalog (#127 I/O, #128 system
// actions): every op the socket gateway serves that is NOT the pure
// layout-state machine in gatewayOps.ts. Layout ops read and write the
// workspace tree; THESE ops touch the world outside it — live PTY handles,
// the OS window, the system opener, the updater, the factory reset.
//
// The module is dependency-injected so the parity fixture suite can drive
// the whole catalog without a running app: WorkspaceShell passes the real
// Tauri invoke / window / opener / updater adapters; tests pass fakes.
// The switch bodies are the EXACT ones the component served before the
// extraction — the wiring is the only thing that moved.

import { leafIds } from './PaneLayout'
import { resolveSendTarget } from './gatewayOps'
import {
  applySettingByKey,
  readAllSettings,
  readSettingByKey,
  SETTING_KEYS,
  type Settings,
} from './settings'
import { applyBranchAnswers, branchDirsByTab } from './tabBranch'
import { localPtyIds } from './tabPorts'
import {
  runCheck,
  runInstall,
  UpdateFlowError,
  type UpdateState,
  type UpdaterApi,
} from './updater'
import type { Tab, WorkspaceState } from './workspaces'

/// A panel's backend handle, mirroring WorkspaceShell's PtyEntry map values.
export type PtyEntry = { kind: 'local' | 'ssh'; id: number }

/// How long the window waits before performing a process-level action the
/// answer promised (relaunch after install/reset, closing the window): the
/// answer must reach the CLI through the gateway round trip first.
const PERFORM_GRACE_MS = 800

export interface LiveOpDeps {
  invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>
  state(): WorkspaceState
  settings(): Settings
  /// The live backend handle of a panel, or null when its shell is not up.
  liveShell(panelId: string | null): PtyEntry | null
  /// The SAME applier a settings toggle click uses (the bridge persists).
  applySettingPatch(patch: Partial<Settings>): void
  relaunch(): void
  /// The OS window API, or null where it is unavailable (tests, exotic
  /// webviews) — window ops answer the honest ioError then.
  window():
    | {
        minimize(): Promise<void>
        toggleMaximize(): Promise<void>
        close(): Promise<void>
      }
    | null
  /// The tab fullscreen (chrome-hide) mode's setter — the SAME one the tab
  /// menu drives; null clears it. The mode is TRANSIENT UI state (not
  /// persisted), which is why the op is live-only.
  setFullscreen(tabId: string | null): void
  openPath(path: string): Promise<void>
  revealItemInDir(path: string): Promise<void>
  openUrl(url: string): Promise<void>
  updaterApi: UpdaterApi
}

/// The live op catalog (#128): the fixture source for the parity suite —
/// every op this module answers, named once. Order mirrors the switch.
export const LIVE_OPS: ReadonlyArray<string> = [
  // #127 — terminal I/O and the app-side session mapping
  'tabs.send',
  'settings.get',
  'settings.set',
  'settings.open',
  'sessions.list',
  'sessions.write',
  'sessions.resize',
  'sessions.kill',
  'sessions.status',
  'sessions.create',
  'sessions.subscribe',
  'sessions.unsubscribe',
  // #128 — system actions (full 1:1 with the UI)
  'update.check',
  'update.install',
  'reset.run',
  'open.folder',
  'open.port',
  'window.minimize',
  'window.toggle-maximize',
  'window.close',
  'tabs.ports',
  'tabs.branches',
  // The plan Appendix's `tabs fullscreen on/off` (chrome-hide): the last
  // inventory row that had no command — added with the parity fixture.
  'tabs.fullscreen',
]

/// One `UpdateState` as the check result doc carries it — the same states
/// the Settings row renders, machine-readable.
function updateStateDoc(state: UpdateState): Record<string, unknown> {
  switch (state.kind) {
    case 'available':
      return { state: 'available', version: state.version }
    case 'error':
      return { state: 'error', errorKind: state.errorKind, message: state.message }
    default:
      return { state: state.kind }
  }
}

/// Handle one gateway op request. Returns TRUE when the op is ours (the
/// caller must not run it through the layout machine); the answer goes out
/// through `gateway_op_respond` exactly as before the extraction.
export function handleLiveOp(
  deps: LiveOpDeps,
  id: number,
  op: string,
  p: Record<string, unknown>,
): boolean {
  const answer = (id: number, payload: Record<string, unknown>) => {
    void deps.invoke('gateway_op_respond', { id, payload }).catch((e) =>
      console.error('gateway_op_respond failed:', e),
    )
  }
  const opErr = (code: string, message: string, next: string[] = []) => ({
    ok: false,
    error: { code, message, next, retryable: false },
  })
  const strParam = (k: string): string | null =>
    typeof p[k] === 'string' && (p[k] as string) !== '' ? (p[k] as string) : null
  const liveShell = (panelId: string | null) => {
    if (panelId == null) return null
    return deps.liveShell(panelId)
  }
  /// The answer-then-perform shape for the actions that end the process or
  /// the window: the result doc describes what WILL happen, the app does it
  /// after the grace window (the CLI has read its answer by then).
  const performAfter = (action: () => void) => {
    setTimeout(action, PERFORM_GRACE_MS)
  }
  /// An UpdateFlowError (the updater state machine's failure shape) as the
  /// wire answer: the quiet no-release state is a RESULT (the honest "no
  /// feed yet"), every error state is the enumerated error object with the
  /// classified message — the same sentence the Settings row renders.
  const updateAnswer = (id: number, err: unknown) => {
    if (err instanceof UpdateFlowError && err.state.kind === 'no-release') {
      answer(id, { ok: true, result: { update: { state: 'no-release' } } })
      return
    }
    const message =
      err instanceof UpdateFlowError
        ? err.message
        : err instanceof Error
          ? err.message
          : String(err)
    answer(id, opErr('ioError', message))
  }
  switch (op) {
    case 'tabs.send': {
      const tabId = strParam('tab')
      const text = typeof p['text'] === 'string' ? (p['text'] as string) : ''
      if (tabId == null || text === '') {
        answer(id, opErr('badParams', 'params need "tab" (a tab id) and non-empty "text"'))
        return true
      }
      // The SAME resolution the op-mapping suite pins (focused panel of
      // the active tab; the tab's first panel otherwise).
      const target = resolveSendTarget(deps.state(), tabId)
      if (target == null) {
        answer(
          id,
          opErr(
            'tabNotFound',
            `no tab with id "${tabId}"`,
            ['tab ids come from umux workspaces list --json or a create echo'],
          ),
        )
        return true
      }
      const panelId = target.panelId
      const entry = liveShell(panelId)
      if (entry == null) {
        answer(
          id,
          opErr(
            'paneNotFound',
            'that tab has no live shell to write to (open its workspace first)',
            ['open the workspace: umux workspaces open --json'],
          ),
        )
        return true
      }
      // "lands in that panel's shell and executes": a bare command gets
      // its Enter. Multi-line input passes through verbatim.
      const data = text.endsWith('\n') ? text : `${text}\n`
      deps
        .invoke(entry.kind === 'ssh' ? 'ssh_write' : 'pty_write', {
          id: entry.id,
          data,
        })
        .then(() =>
          answer(id, {
            ok: true,
            result: { sent: { tab: target.tabId, panel: panelId } },
          }),
        )
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'settings.get': {
      const key = strParam('key')
      const s = deps.settings()
      if (key == null) {
        answer(id, { ok: true, result: { settings: readAllSettings(s) } })
        return true
      }
      const value = readSettingByKey(s, key)
      if (value == null) {
        answer(
          id,
          opErr('settingNotFound', `unknown setting "${key}"`, [
            `settings: ${SETTING_KEYS.join(', ')}`,
          ]),
        )
        return true
      }
      answer(id, { ok: true, result: { key, value } })
      return true
    }
    case 'settings.set': {
      const key = strParam('key')
      const value = typeof p['value'] === 'string' ? (p['value'] as string) : ''
      if (key == null) {
        answer(id, opErr('badParams', 'params need "key" and "value"'))
        return true
      }
      const outcome = applySettingByKey(key, value)
      if (!outcome.ok) {
        answer(id, { ok: false, error: outcome.error })
        return true
      }
      // The SAME path a toggle click takes — outcomes cannot diverge.
      deps.applySettingPatch(outcome.patch)
      answer(id, { ok: true, result: { set: { key, value: outcome.value } } })
      return true
    }
    case 'settings.open': {
      deps
        .invoke('open_settings_file')
        .then(() => answer(id, { ok: true, result: { opened: true } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'sessions.list': {
      // The app's LIVE panels — one row per mounted shell (the daemon
      // counterpart lists its registry sessions; same idea, same owner).
      const s = deps.state()
      const sessions: unknown[] = []
      for (const ws of s.workspaces) {
        const tabs = ws.tabs ?? []
        for (let i = 0; i < tabs.length; i++) {
          const tab = tabs[i]
          for (const panelId of leafIds(tab.layout)) {
            if (!deps.liveShell(panelId)) continue
            const cwd =
              ws.panels?.find((pn) => pn.id === panelId)?.workingDirectory ?? null
            sessions.push({
              id: panelId,
              title: tab.name ?? `Tab ${i + 1}`,
              cwd,
              workspace: ws.name,
              workspaceId: ws.id,
              tabId: tab.id,
              cols: null,
              rows: null,
              attachedClients: 0,
            })
          }
        }
      }
      answer(id, { ok: true, result: { sessions, truncated: false } })
      return true
    }
    case 'sessions.write': {
      const panelId = strParam('id')
      const data = typeof p['data'] === 'string' ? (p['data'] as string) : ''
      const entry = liveShell(panelId)
      if (entry == null || data === '') {
        answer(id, opErr('sessionNotFound', `no live panel with id "${panelId ?? ''}"`, ['list the live panels: umux sessions list --json']))
        return true
      }
      deps
        .invoke(entry.kind === 'ssh' ? 'ssh_write' : 'pty_write', { id: entry.id, data })
        .then(() => answer(id, { ok: true, result: { written: true } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'sessions.resize': {
      const panelId = strParam('id')
      const cols = typeof p['cols'] === 'number' ? p['cols'] : null
      const rows = typeof p['rows'] === 'number' ? p['rows'] : null
      const entry = liveShell(panelId)
      if (entry == null || cols == null || rows == null) {
        answer(id, opErr('sessionNotFound', `no live panel with id "${panelId ?? ''}"`, ['list the live panels: umux sessions list --json']))
        return true
      }
      deps
        .invoke(entry.kind === 'ssh' ? 'ssh_resize' : 'pty_resize', { id: entry.id, cols, rows })
        .then(() => answer(id, { ok: true, result: { resized: true } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'sessions.kill': {
      const panelId = strParam('id')
      const entry = liveShell(panelId)
      if (entry == null) {
        answer(id, opErr('sessionNotFound', `no live panel with id "${panelId ?? ''}"`, ['list the live panels: umux sessions list --json']))
        return true
      }
      deps
        .invoke(entry.kind === 'ssh' ? 'ssh_close' : 'pty_close', { id: entry.id })
        .then(() => answer(id, { ok: true, result: { killed: true } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'sessions.status': {
      const panelId = strParam('id')
      const entry = liveShell(panelId)
      if (entry == null) {
        answer(id, opErr('sessionNotFound', `no live panel with id "${panelId ?? ''}"`, []))
        return true
      }
      answer(id, {
        ok: true,
        result: { id: panelId, kind: entry.kind, attachedClients: 0 },
      })
      return true
    }
    case 'sessions.create': {
      // The app spawns terminals through the UI, never headless — the
      // guidance IS the answer (issue #127).
      answer(
        id,
        opErr(
          'badParams',
          'the umux app does not spawn headless sessions — create terminals through the UI or the CLI against the app',
          ['open a terminal: umux tabs create --workspace <name>', 'bring the daemon world back: umux attach'],
        ),
      )
      return true
    }
    case 'sessions.subscribe':
    case 'sessions.unsubscribe': {
      // Streaming a live panel's output runs over the daemon's registry
      // — the app gateway has no stream to hand out, and pretending
      // otherwise would leave the subscriber silent forever.
      answer(
        id,
        opErr(
          'badParams',
          'live-panel streaming requires the daemon — bring it back with umux attach',
          ['bring the daemon world back: umux attach'],
        ),
      )
      return true
    }
    // --- #128 system actions (full 1:1 with the UI, PO 2026-11-02) --------
    case 'update.check': {
      // The updater is the Settings row's OWN machine (src/updater.ts) —
      // the pre-flight answers first, so offline / not-configured /
      // no-release read as the row reads, never as a plugin crash.
      runCheck(deps.updaterApi)
        .then((res) =>
          answer(id, { ok: true, result: { update: updateStateDoc(res.state) } }),
        )
        .catch((e) => updateAnswer(id, e))
      return true
    }
    case 'update.install': {
      // Install = the Settings button's whole flow: check (the resource is
      // what the row kept), download + apply, then the relaunch — the
      // answer DESCRIBES the relaunch, the app performs it after the
      // grace window.
      runCheck(deps.updaterApi)
        .then(async (res) => {
          if (res.update == null) {
            answer(id, {
              ok: true,
              result: {
                installed: false,
                update: updateStateDoc(res.state),
              },
            })
            return
          }
          await runInstall(deps.updaterApi, res.update, () => {})
          answer(id, {
            ok: true,
            result: {
              installed: true,
              version: res.update.version,
              relaunchScheduled: true,
            },
          })
          performAfter(() => deps.relaunch())
        })
        .catch((e) => updateAnswer(id, e))
      return true
    }
    case 'reset.run': {
      // The Settings arm's exact two-step semantics: wipe EVERY umux state
      // file (`reset_all` — the app command the reset screen invokes),
      // then relaunch first-run clean. The answer describes the relaunch.
      deps
        .invoke('reset_all')
        .then(() => {
          answer(id, {
            ok: true,
            result: { reset: true, relaunchScheduled: true },
          })
          performAfter(() => deps.relaunch())
        })
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'open.folder': {
      // The folder line's double-click: openPath shows the folder's
      // CONTENTS; when the opener refuses (a vanished dir, an ACL) the
      // UI's fallback reveals the folder in its parent — same order.
      const folder = strParam('target')
      if (folder == null) {
        answer(id, opErr('badParams', 'params need "target" (a folder path)'))
        return true
      }
      deps
        .openPath(folder)
        .catch(() => deps.revealItemInDir(folder))
        .then(() => answer(id, { ok: true, result: { opened: folder } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'open.port': {
      // The tooltip's port click minus the clipboard copy (a UI
      // convenience an agent's clipboard must not suffer): the same
      // http://localhost URL into the system browser, no protocol
      // detection (PO decision 2026-09-01).
      const tab = strParam('tab')
      const port = p['port']
      if (tab == null || typeof port !== 'number' || !Number.isInteger(port) || port <= 0 || port > 65535) {
        answer(
          id,
          opErr('badParams', 'params need "tab" (a tab id) and integer "port" (1–65535)'),
        )
        return true
      }
      const url = `http://localhost:${port}`
      deps
        .openUrl(url)
        .then(() => answer(id, { ok: true, result: { opened: url, tab } }))
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'window.minimize':
    case 'window.toggle-maximize': {
      const win = deps.window()
      if (win == null) {
        answer(id, opErr('ioError', 'the window API is unavailable here'))
        return true
      }
      const act = op === 'window.minimize' ? win.minimize() : win.toggleMaximize()
      act
        .then(() =>
          answer(id, {
            ok: true,
            result:
              op === 'window.minimize' ? { minimized: true } : { toggled: true },
          }),
        )
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'window.close': {
      const win = deps.window()
      if (win == null) {
        answer(id, opErr('ioError', 'the window API is unavailable here'))
        return true
      }
      // The answer goes out first — the window closes after the grace
      // window, once the CLI has read it.
      answer(id, { ok: true, result: { closing: true } })
      performAfter(() => void win.close())
      return true
    }
    case 'tabs.ports': {
      // The hover tooltip's query, addressed by tab id: LOCAL panel
      // handles only (an SSH panel's local process is just the ssh
      // client), the backend's ascending deduplicated list back — the
      // exact data the tooltip shows.
      const tabId = strParam('tab')
      let tab: Tab | undefined
      for (const ws of deps.state().workspaces) {
        const found = ws.tabs?.find((t) => t.id === tabId)
        if (found != null) {
          tab = found
          break
        }
      }
      if (tab == null) {
        answer(
          id,
          opErr(
            'tabNotFound',
            `no tab with id "${tabId ?? ''}"`,
            ['tab ids come from umux workspaces list --json or a create echo'],
          ),
        )
        return true
      }
      // The tooltip's exact query shape: the tab's LOCAL panel handles in
      // tree order, one batch row for the backend.
      const handles = new Map<string, PtyEntry>()
      for (const panelId of leafIds(tab.layout)) {
        const entry = deps.liveShell(panelId)
        if (entry != null) handles.set(panelId, entry)
      }
      const ids = localPtyIds(leafIds(tab.layout), handles)
      if (ids.length === 0) {
        answer(id, { ok: true, result: { tab: tab.id, ports: [] } })
        return true
      }
      deps
        .invoke<Array<{ tabId: string; ports: number[] }>>('tab_ports', {
          tabs: [{ tabId: tab.id, ptyIds: ids }],
        })
        .then((answers) => {
          const ports = Array.isArray(answers) ? answers[0]?.ports ?? [] : []
          answer(id, { ok: true, result: { tab: tab.id, ports } })
        })
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'tabs.branches': {
      // The sidebar's branch labels for one tab: the SAME per-panel
      // mapping (branchDirsByTab) and the SAME session-restore gate the
      // labels render under; answers merge into one panel list.
      const tabId = strParam('tab')
      let found: { tab: Tab } | undefined
      for (const ws of deps.state().workspaces) {
        const t = ws.tabs?.find((x) => x.id === tabId)
        if (t != null) {
          found = { tab: t }
          break
        }
      }
      if (found == null) {
        answer(
          id,
          opErr(
            'tabNotFound',
            `no tab with id "${tabId ?? ''}"`,
            ['tab ids come from umux workspaces list --json or a create echo'],
          ),
        )
        return true
      }
      const restoreEnabled =
        readSettingByKey(deps.settings(), 'session-restore-enabled') !== 'false'
      const entries = branchDirsByTab(deps.state(), restoreEnabled)[found.tab.id] ?? []
      const dirs = [
        ...new Set(entries.map((e) => e.dir).filter((d): d is string => d != null)),
      ]
      if (dirs.length === 0) {
        answer(id, {
          ok: true,
          result: {
            tab: found.tab.id,
            panels: entries.map((e) => ({
              panelId: e.panelId,
              branch: null,
              focused: e.focused,
            })),
          },
        })
        return true
      }
      deps
        .invoke<Array<{ dir: string; branch: string | null }>>('git_branches', { dirs })
        .then((answers) => {
          const labels = applyBranchAnswers({}, Array.isArray(answers) ? answers : [])
          answer(id, {
            ok: true,
            result: {
              tab: found.tab.id,
              panels: entries.map((e) => ({
                panelId: e.panelId,
                branch: e.dir != null ? labels[e.dir] ?? null : null,
                focused: e.focused,
              })),
            },
          })
        })
        .catch((e) => answer(id, opErr('ioError', String(e))))
      return true
    }
    case 'tabs.fullscreen': {
      // The tab menu's fullscreen chrome-hide (#128, plan Appendix): a
      // TRANSIENT view state — the workspace+tab whose terminal owns the
      // whole window — so the op is live-only by nature. `on` enters the
      // mode on one tab, `off` leaves it (the ESC path).
      const tabId = strParam('tab')
      const on = p['on']
      if (tabId == null || typeof on !== 'boolean') {
        answer(id, opErr('badParams', 'params need "tab" (a tab id) and boolean "on"'))
        return true
      }
      if (!on) {
        deps.setFullscreen(null)
        answer(id, { ok: true, result: { fullscreen: { tab: tabId, on: false } } })
        return true
      }
      // Entering: the tab must exist AND its workspace must be open — the
      // same precondition the menu item sits under (the mode drops when the
      // workspace closes; a closed workspace has no surface to hide into).
      let wsId: string | null = null
      for (const ws of deps.state().workspaces) {
        if (ws.tabs?.some((t) => t.id === tabId)) {
          wsId = ws.id
          break
        }
      }
      if (wsId == null) {
        answer(
          id,
          opErr('tabNotFound', `no tab with id "${tabId}"`, [
            'tab ids come from umux workspaces list --json or a create echo',
          ]),
        )
        return true
      }
      if (!deps.state().openIds.includes(wsId)) {
        answer(
          id,
          opErr(
            'workspaceNotFound',
            'that tab\'s workspace is not open — fullscreen needs its surface',
            ['open the workspace: umux workspaces open --json'],
          ),
        )
        return true
      }
      deps.setFullscreen(tabId)
      answer(id, { ok: true, result: { fullscreen: { tab: tabId, on: true } } })
      return true
    }
    default:
      return false
  }
}
