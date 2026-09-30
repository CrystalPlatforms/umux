// bundle-cli.mjs — puts the `umux` CLI and the `umux-storestation` daemon
// binaries where the Tauri bundler expects sidecars (issues #64 and #90).
//
// `externalBin: ["binaries/umux", "binaries/umux-storestation"]` in
// tauri.conf.json makes every installer carry both: .deb → /usr/bin/{umux,
// umux-storestation}, NSIS → install dir (exposed on PATH by
// installer-hooks.nsh), macOS .app → Contents/MacOS/ (beside the app
// binary). The bundler looks for files named `<name>-<target triple>` next
// to this script's parent (src-tauri/binaries/), and this script is the
// `beforeBuildCommand` step that creates them:
//
//   1. Read the triple from TAURI_ENV_TARGET_TRIPLE (set by the Tauri CLI
//      for build hooks; falls back to the host triple for manual runs).
//   2. `universal-apple-darwin` is not a real Rust target — build BOTH
//      darwin triples and join them with `lipo` (same as the app build).
//   3. Anything else: one plain `cargo build --release -p umux
//      -p umux-storestation --target`.
//
// The daemon rides beside the CLI everywhere the CLI goes (issue #90):
// enabling Storestation never means installing something extra, and a
// normal app update replaces the daemon too — its version stays locked to
// the workspace version.
//
// The binaries/ directory is gitignored — it is a build artifact, like
// target/ itself.

import { execSync } from 'node:child_process'
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// Plain-stream copy instead of copyFileSync/fcopyfile (build fix 2026-09-07):
// fcopyfile preserves APFS `com.apple.decmpfs` compression metadata, and the
// tauri-build script's Rust `fs::copy` (fclonefileat path) then fails on that
// sidecar with "Operation not permitted" whenever the file was produced by a
// different process lineage (macOS provenance). A read+write copy produces a
// plain data file with no decmpfs, which fs::copy always accepts.
// writeFileSync only keeps the executable bit when the destination ALREADY
// exists — a fresh CI checkout would ship 644 sidecars, so the exec bit is
// set explicitly (issue #90: both sidecars must run from every installer).
function plainCopy(from, to) {
  writeFileSync(to, readFileSync(from))
  chmodSync(to, 0o755)
}

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

// Sidecars shipped in every installer (#64 CLI, #90 daemon) — one cargo
// build produces both, and both travel the exact same copy pipeline.
const SIDECARS = [
  { pkg: 'umux', name: 'umux' },
  { pkg: 'umux-storestation', name: 'umux-storestation' },
]

function copySidecar(pkg, fromRel, destName) {
  const from = path.join(srcTauri, fromRel)
  if (!existsSync(from)) {
    throw new Error(`[bundle-cli] expected ${pkg} binary at ${from} — cargo build failed?`)
  }
  mkdirSync(outDir, { recursive: true })
  const to = path.join(outDir, destName)
  plainCopy(from, to)
  console.log(`[bundle-cli] sidecar ready: ${to}`)
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.TAURI_TARGET_TRIPLE ?? hostTriple()
const exe = triple.includes('windows') ? '.exe' : ''

if (triple === 'universal-apple-darwin') {
  // Same treatment the app build gets: two single-arch cargo builds joined
  // into one fat binary. Stale parts are removed first so an interrupted
  // earlier run can never be lipo'd into the artifact.
  for (const arch of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) {
    run(`cargo build --release -p umux -p umux-storestation --target ${arch}`)
  }
  mkdirSync(outDir, { recursive: true })
  for (const { pkg, name } of SIDECARS) {
    const fat = path.join(outDir, `${name}-universal-apple-darwin`)
    rmSync(fat, { force: true })
    run(
      'lipo -create ' +
        `target/aarch64-apple-darwin/release/${name} ` +
        `target/x86_64-apple-darwin/release/${name} ` +
        `-output ${path.relative(srcTauri, fat)}`,
    )
    // The BUNDLER wants the fat binary, but tauri-build validates the sidecar
    // per single-arch triple while it compiles each half of the universal app
    // (CI failure 2026-08-31: "resource path binaries/umux-aarch64-apple-darwin
    // doesn't exist") — so the lipo output is published under all three names.
    for (const arch of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) {
      plainCopy(fat, path.join(outDir, `${name}-${arch}`))
    }
    console.log(`[bundle-cli] ${pkg} sidecar ready: ${fat} (+ per-arch copies)`)
  }
} else {
  run(`cargo build --release -p umux -p umux-storestation --target ${triple}`)
  for (const { pkg, name } of SIDECARS) {
    copySidecar(pkg, path.join('target', triple, 'release', `${name}${exe}`), `${name}-${triple}${exe}`)
  }
}
