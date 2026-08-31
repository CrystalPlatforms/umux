// updater — the update-check state machine (issue #66).
//
// Deep module: a tiny surface (runCheck / runInstall + a state union) hiding
// everything about the Tauri updater plugin, its error strings, and progress
// aggregation. The plugin boundary sits behind UpdaterApi so the behaviorally
// interesting part — which state each outcome maps to — is unit-testable with
// fakes; `defaultUpdaterApi` is the thin adapter on the plugin boundary.
//
// Source of updates (zero-cost policy): GitHub Releases ONLY, via the
// latest.json endpoint configured in tauri.conf.json. No update server.
//
// Assumptions encoded by the tests:
//  - runCheck maps the plugin's `Update | null` to exactly one settled state:
//    available (newer release) / up-to-date (null) — plus error states from
//    classifyUpdateError on failure.
//  - classifyUpdateError pattern-matches the plugin's error STRINGS (the
//    plugin exposes no error codes). Representative strings are pinned by
//    tests; unmatched strings fall back to a generic error that still
//    renders. The five kinds drive both the wording and the styling.
//  - 'no-release' (a release without latest.json) and 'not-configured'
//    (placeholder pubkey before the HITL step) are QUIET states at startup:
//    the caller decides to ignore them there and only show them in Settings.
//  - runInstall aggregates Started/Progress events into received/total and
//    relaunches via the api after a successful install (one click = download
//    + apply + restart). Signature rejection surfaces the update's error as
//    kind 'signature' — the plugin refuses a bad signature itself; we only
//    render it.
//  - NOT tested here: the real plugin round-trip (needs a live release),
//    the UI rendering (SettingsDialog/shell glue verified by Adam).

import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { invoke } from '@tauri-apps/api/core'

// --- The state the UI renders -------------------------------------------------

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'up-to-date' }
  // A release exists but has no latest.json yet (graceful no-op — never a
  // crash, at startup fully silent).
  | { kind: 'no-release' }
  | { kind: 'available'; version: string }
  | { kind: 'downloading'; received: number; total: number | null }
  | { kind: 'error'; errorKind: UpdateErrorKind; message: string }

export type UpdateErrorKind =
  | 'offline'
  | 'no-release'
  | 'signature'
  | 'not-configured'
  | 'unknown'

// --- The plugin boundary -------------------------------------------------------

// Structural stand-in for the plugin's `Update` class (only what we use).
export interface UpdateResource {
  version: string
  downloadAndInstall(
    onEvent?: (event: DownloadEvent) => void,
  ): Promise<void>
}

export type DownloadEvent =
  | { event: 'Started'; data: { contentLength?: number } }
  | { event: 'Progress'; data: { chunkLength: number } }
  | { event: 'Finished' }

// The backend pre-flight verdict (updater_status command, updater_probe.rs).
// 'ok' is the only verdict under which the plugin check runs at all.
export type PreflightStatus = 'unconfigured' | 'offline' | 'no-release' | 'ok'

export interface UpdaterApi {
  // Asked FIRST, every time: it disambiguates what the plugin cannot —
  // "unconfigured" (no signer key yet) and "no-release" (feed not published)
  // are both quiet, expected states, not errors.
  preflight(): Promise<PreflightStatus>
  check(): Promise<UpdateResource | null>
  install(update: UpdateResource, onEvent: (event: DownloadEvent) => void): Promise<void>
  relaunch(): void
}

// The real adapter: the Tauri command + plugins. Fakes replace this in tests.
export const defaultUpdaterApi: UpdaterApi = {
  preflight: async () => {
    const status = await invoke<string>('updater_status')
    return status as PreflightStatus
  },
  check: () => check() as unknown as Promise<UpdateResource | null>,
  install: (update, onEvent) => update.downloadAndInstall(onEvent),
  relaunch: () => void relaunch(),
}

// The canonical sentences — one definition each, used by both the classifier
// and the pre-flight map so the wording can never drift apart.
const NOT_CONFIGURED_MESSAGE =
  'Updates are not configured yet: the app is missing its signing public key (see README, "Updates").'
const OFFLINE_MESSAGE = 'You appear to be offline. Check your connection and try again.'
const NO_RELEASE_MESSAGE = 'No update information was published yet.'

// The pre-flight verdicts that settle the check without calling the plugin.
const PREFLIGHT_SETTLED: Record<Exclude<PreflightStatus, 'ok'>, ClassifiedError> = {
  unconfigured: { kind: 'not-configured', message: NOT_CONFIGURED_MESSAGE },
  offline: { kind: 'offline', message: OFFLINE_MESSAGE },
  'no-release': { kind: 'no-release', message: NO_RELEASE_MESSAGE },
}

// --- Error classification (pure) ----------------------------------------------

export interface ClassifiedError {
  kind: UpdateErrorKind
  message: string
}

// Pattern-match the plugin's error text. Order matters: network phrasing
// ("Could not fetch a valid response") is checked as offline BEFORE the looser
// "fetch" hit would match no-release; the 404/no-asset patterns are distinct.
export function classifyUpdateError(raw: unknown): ClassifiedError {
  const text =
    raw instanceof Error ? raw.message : typeof raw === 'string' ? raw : String(raw)

  const has = (...patterns: RegExp[]) => patterns.some((p) => p.test(text))

  // Placeholder pubkey / missing updater config — the HITL step hasn't run.
  if (has(/public key/i)) {
    return { kind: 'not-configured', message: NOT_CONFIGURED_MESSAGE }
  }
  // Signature verification failed — the plugin rejects tampered/unsigned
  // bundles before installing anything. Enforced by the plugin; we render it.
  if (has(/signature|verif(y|ication)/i)) {
    return {
      kind: 'signature',
      message: 'The update was rejected: its signature could not be verified.',
    }
  }
  // Network shapes are checked BEFORE the missing-manifest shapes on purpose:
  // transport errors often embed the full endpoint URL (which contains
  // "latest.json") and must still read as offline, not as a missing release.
  if (
    has(
      /network/i,
      /offline/i,
      /\bdns\b/i,
      /timed?\s?out/i,
      /connection/i,
      /error sending request/i,
      /could not fetch/i,
    )
  ) {
    return {
      kind: 'offline',
      message: OFFLINE_MESSAGE,
    }
  }
  // A published release whose latest.json (or its platform asset) is missing.
  if (has(/404|status code 40\d|no update|latest\.json/i)) {
    return { kind: 'no-release', message: NO_RELEASE_MESSAGE }
  }
  return {
    kind: 'unknown',
    message: `Update check failed: ${text}`,
  }
}

// --- The flows ------------------------------------------------------------------

export class UpdateFlowError extends Error {
  readonly state: UpdateState

  constructor(classified: ClassifiedError) {
    super(classified.message)
    this.name = 'UpdateFlowError'
    this.state =
      classified.kind === 'no-release'
        ? { kind: 'no-release' }
        : { kind: 'error', errorKind: classified.kind, message: classified.message }
  }
}

export interface CheckResult {
  state: UpdateState
  update: UpdateResource | null
}

// One quiet check. Resolves to a settled state (never 'checking' — the caller
// renders that while the promise is in flight). The pre-flight answers first;
// only 'ok' reaches the plugin, so a missing feed can never be misread as
// "offline" again.
export async function runCheck(api: UpdaterApi): Promise<CheckResult> {
  const pre = await api.preflight().catch((err) => {
    console.error('[updater] preflight failed:', err)
    return 'unknown' as const
  })
  if (pre !== 'ok' && pre in PREFLIGHT_SETTLED) {
    throw new UpdateFlowError(PREFLIGHT_SETTLED[pre as Exclude<PreflightStatus, 'ok'>])
  }

  let update: UpdateResource | null
  try {
    update = await api.check()
  } catch (raw) {
    // Log the raw plugin error before classification — classification is a
    // best-effort translation, and this line is what makes future mislabels
    // diagnosable from the devtools console in one glance.
    console.error('[updater] plugin check failed:', raw)
    throw new UpdateFlowError(classifyUpdateError(raw))
  }
  if (update == null) {
    return { state: { kind: 'up-to-date' }, update: null }
  }
  return { state: { kind: 'available', version: update.version }, update }
}

// Download + apply, reporting progress as (received, total|null). The caller
// relaunches through api.relaunch() AFTER this resolves — one user click
// covers download + apply + restart.
export async function runInstall(
  api: UpdaterApi,
  update: UpdateResource,
  onProgress: (received: number, total: number | null) => void,
): Promise<void> {
  let received = 0
  let total: number | null = null
  try {
    await api.install(update, (event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength ?? null
        onProgress(0, total)
      } else if (event.event === 'Progress') {
        received += event.data.chunkLength
        onProgress(received, total)
      }
    })
  } catch (raw) {
    console.error('[updater] install failed:', raw)
    throw new UpdateFlowError(classifyUpdateError(raw))
  }
}

// --- Rendering helpers (pure) ---------------------------------------------------

function mb(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

// The one-line download readout: "12.3 MB of 45.6 MB", or "12.3 MB downloaded"
// while the total is still unknown.
export function downloadProgressText(received: number, total: number | null): string {
  return total == null ? `${mb(received)} downloaded` : `${mb(received)} of ${mb(total)}`
}
