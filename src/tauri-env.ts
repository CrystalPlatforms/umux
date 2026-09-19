// Tauri runtime detection (browser-mode guard).
//
// `vite dev` opened in a plain browser has no Tauri runtime: the
// `__TAURI_INTERNALS__` global that every @tauri-apps/* entry point
// (invoke, listen, getCurrentWindow, ...) reads is undefined, so the first
// effect that touches it crashes the whole tree (reading 'metadata' /
// 'invoke' / 'transformCallback'). App.tsx checks this BEFORE mounting
// WorkspaceShell and shows a notice instead — the shell only makes sense
// inside the desktop window (run `yarn tauri dev` for development).
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}
