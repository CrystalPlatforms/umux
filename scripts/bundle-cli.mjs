// bundle-cli.mjs — puts the `umux` CLI binary where the Tauri bundler
// expects a sidecar (issue #64).
//
// `externalBin: ["binaries/umux"]` in tauri.conf.json makes every installer
// carry the CLI: .deb → /usr/bin/umux, NSIS → install dir (exposed on PATH
// by installer-hooks.nsh), macOS .app → Contents/MacOS/umux (beside the app
// binary). The bundler looks for the file named `umux-<target triple>` next
// to this script's parent (src-tauri/binaries/), and this script is the
// `beforeBuildCommand` step that creates it:
//
//   1. Read the triple from TAURI_ENV_TARGET_TRIPLE (set by the Tauri CLI
//      for build hooks; falls back to the host triple for manual runs).
//   2. `universal-apple-darwin` is not a real Rust target — build BOTH
//      darwin triples and join them with `lipo` (same as the app build).
//   3. Anything else: one plain `cargo build --release -p umux --target`.
//
// The binaries/ directory is gitignored — it is a build artifact, like
// target/ itself.

import { execSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const srcTauri = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri')
const outDir = path.join(srcTauri, 'binaries')

function hostTriple() {
  const arch = { arm64: 'aarch64', x64: 'x86_64' }[process.arch] ?? process.arch
  const os = {
    darwin: 'apple-darwin',
    linux: 'unknown-linux-gnu',
    win32: 'pc-windows-msvc',
  }[process.platform]
  if (os == null) throw new Error(`unsupported host platform: ${process.platform}`)
  return `${arch}-${os}`
}

function run(cmd, { cwd = srcTauri } = {}) {
  console.log(`[bundle-cli] ${cmd}`)
  execSync(cmd, { cwd, stdio: 'inherit' })
}

function copyCli(fromRel, destName) {
  const from = path.join(srcTauri, fromRel)
  if (!existsSync(from)) {
    throw new Error(`[bundle-cli] expected CLI binary at ${from} — cargo build failed?`)
  }
  mkdirSync(outDir, { recursive: true })
  const to = path.join(outDir, destName)
  copyFileSync(from, to)
  console.log(`[bundle-cli] sidecar ready: ${to}`)
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.TAURI_TARGET_TRIPLE ?? hostTriple()

if (triple === 'universal-apple-darwin') {
  // Same treatment the app build gets: two single-arch cargo builds joined
  // into one fat binary. Stale parts are removed first so an interrupted
  // earlier run can never be lipo'd into the artifact.
  for (const arch of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) {
    run(`cargo build --release -p umux --target ${arch}`)
  }
  mkdirSync(outDir, { recursive: true })
  const fat = path.join(outDir, 'umux-universal-apple-darwin')
  rmSync(fat, { force: true })
  run(
    'lipo -create ' +
      'target/aarch64-apple-darwin/release/umux ' +
      'target/x86_64-apple-darwin/release/umux ' +
      `-output ${path.relative(srcTauri, fat)}`,
  )
  // The BUNDLER wants the fat binary, but tauri-build validates the sidecar
  // per single-arch triple while it compiles each half of the universal app
  // (CI failure 2026-08-31: "resource path binaries/umux-aarch64-apple-darwin
  // doesn't exist") — so the lipo output is published under all three names.
  for (const name of [
    'umux-aarch64-apple-darwin',
    'umux-x86_64-apple-darwin',
  ]) {
    copyFileSync(fat, path.join(outDir, name))
  }
  console.log(`[bundle-cli] sidecar ready: ${fat} (+ per-arch copies)`)
} else {
  run(`cargo build --release -p umux --target ${triple}`)
  const exe = triple.includes('windows') ? '.exe' : ''
  copyCli(path.join('target', triple, 'release', `umux${exe}`), `umux-${triple}${exe}`)
}
