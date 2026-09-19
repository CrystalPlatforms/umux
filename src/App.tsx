import { WorkspaceShell } from './WorkspaceShell'
import { isTauri } from './tauri-env'

export default function App() {
  // Browser-mode guard: without the Tauri runtime every invoke/listen/
  // getCurrentWindow inside the shell crashes on mount (no
  // `__TAURI_INTERNALS__`). The notice tells a stray browser tab how to
  // actually run the app instead of blank-screening with a stack trace.
  if (!isTauri()) {
    return (
      <div
        style={{
          minHeight: '100vh',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 12,
          background: '#0d1117',
          color: '#c9d1d9',
          fontFamily: 'system-ui, sans-serif',
          padding: 24,
          textAlign: 'center',
        }}
      >
        <h1 style={{ fontSize: 20, margin: 0 }}>umux runs as a desktop app</h1>
        <p style={{ margin: 0, maxWidth: 480, lineHeight: 1.6 }}>
          This browser tab has no Tauri runtime, so the workspace shell cannot
          start here. For development run <code>yarn tauri dev</code>; to use
          the app, launch the installed umux binary.
        </p>
      </div>
    )
  }
  return <WorkspaceShell />
}
