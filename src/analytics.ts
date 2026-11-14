/**
 * Anonymous launch counter (quickupdate 2026-11-14 — Aptabase → CounterAPI).
 * Each production launch increments one public counter — no identifiers,
 * no content, no cookies, just a number going up so we can tell roughly
 * how many people run umux.
 *
 * The counter is PUBLIC by design: umux is an open-source repo and must
 * ship no secrets, so the workspace counter (`crystals-team-5978/umux`) is
 * created public in the counterapi.dev dashboard and the request needs no
 * key. counterapi.dev v2 counts it on `GET /up`; anyone who learns the URL
 * can inflate it — that is accepted (it is only a counter, fixable by hand).
 *
 * Dev builds (`tauri dev`), unit tests and plain-browser runs never send
 * anything: only a packaged launch inside the Tauri shell counts.
 */

const APP_OPEN_URL = "https://api.counterapi.dev/v2/crystals-team-5978/umux/up";

/** Guards against a second count within the same running session. */
let counted = false;

export function trackAppOpen(): void {
  // Vite flags dev server and test runs as DEV — packaged builds only, then.
  if (import.meta.env.DEV) return;
  // Only when the renderer runs inside the Tauri shell, never in a bare
  // browser tab (vite preview etc.).
  if (!("__TAURI_INTERNALS__" in window)) return;
  if (counted) return;
  counted = true;

  // Fire-and-forget: analytics must never delay startup or surface an
  // error — an offline machine just doesn't count.
  fetch(APP_OPEN_URL, { cache: "no-store" }).catch(() => {});
}
