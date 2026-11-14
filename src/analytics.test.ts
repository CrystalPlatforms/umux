//! Anonymous launch counter (quickupdate 2026-11-14, Aptabase → CounterAPI).
//!
//! Assumptions these tests encode:
//! - Only a packaged launch inside the Tauri shell counts: dev builds and
//!   unit tests (Vite `DEV`) and plain-browser runs (no `__TAURI_INTERNALS__`)
//!   stay silent — so neither local development nor localhost scans pollute
//!   the counter.
//! - One running session counts at most once, even if the hook fires twice.
//! - The counter is PUBLIC (no auth — counterapi v2 keeps private counters
//!   behind a Bearer key, but a public workspace counter needs none; umux is
//!   an open-source repo and must ship no secrets), so the request carries
//!   no Authorization header.
//! - A failing network never throws — analytics is fire-and-forget.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type * as Analytics from "./analytics";

const APP_OPEN_URL = "https://api.counterapi.dev/v2/crystals-team-5978/umux/up";
const TAURI = "__TAURI_INTERNALS__";

/** Fresh module per test so the one-shot launch guard resets. */
const load = () => import("./analytics") as Promise<typeof Analytics>;

/** Tauri v2 injects its marker global straight onto the window object. */
const windowBag = () => window as unknown as Record<string, unknown>;

describe("trackAppOpen", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.resetModules();
    fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 200 })));
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
    delete windowBag()[TAURI];
  });

  it("stays silent in dev builds (tauri dev, unit tests)", async () => {
    const { trackAppOpen } = await load();
    trackAppOpen();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("stays silent outside the Tauri shell (plain browser tab)", async () => {
    vi.stubEnv("DEV", false);
    const { trackAppOpen } = await load();
    trackAppOpen();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("counts exactly one app_open against the public umux counter", async () => {
    vi.stubEnv("DEV", false);
    windowBag()[TAURI] = {};
    const { trackAppOpen } = await load();
    trackAppOpen();
    trackAppOpen();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      APP_OPEN_URL,
      expect.objectContaining({ cache: "no-store" }),
    );
    const config = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(config[1].headers).toBeUndefined();
  });

  it("swallows network failures quietly", async () => {
    vi.stubEnv("DEV", false);
    windowBag()[TAURI] = {};
    fetchMock.mockImplementation(() => Promise.reject(new Error("offline")));
    const { trackAppOpen } = await load();
    expect(() => trackAppOpen()).not.toThrow();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
