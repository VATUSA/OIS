// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {fetchUpdate} from "./desktop-update";

const check = vi.fn();
const download = vi.fn();
const downloadAndInstall = vi.fn();
const relaunch = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({check: () => check()}));
vi.mock("@tauri-apps/plugin-process", () => ({relaunch: () => relaunch()}));

/** `can("autoUpdate")` is true only under Tauri, so the tests toggle the same global it reads. */
function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

beforeEach(() => {
  check.mockReset();
  download.mockReset();
  downloadAndInstall.mockReset();
  relaunch.mockReset();
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("fetchUpdate", () => {
  it("does nothing at all on the web build", async () => {
    await expect(fetchUpdate()).resolves.toEqual({state: "idle"});
    expect(check).not.toHaveBeenCalled();
  });

  it("is idle when the app is already current", async () => {
    pretendDesktop();
    check.mockResolvedValue(null);

    await expect(fetchUpdate()).resolves.toEqual({state: "idle"});
  });

  it("reports ready only after the package is downloaded and verified", async () => {
    pretendDesktop();
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall});
    download.mockResolvedValue(undefined);

    await expect(fetchUpdate()).resolves.toEqual({state: "ready", version: "1.2.3"});
    expect(download).toHaveBeenCalledTimes(1);
  });

  it("does NOT report ready when the signature fails to verify", async () => {
    // The updater plugin throws rather than returning a package whose minisign signature doesn't
    // match the pubkey compiled into the app. A tampered update must therefore end as `failed` —
    // never `ready`, because `ready` is what the banner offers the user a restart for.
    pretendDesktop();
    check.mockResolvedValue({version: "9.9.9", download, downloadAndInstall});
    download.mockRejectedValue(new Error("signature verification failed"));

    await expect(fetchUpdate()).resolves.toEqual({state: "failed"});
    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("never restarts the app on its own, even for a good update", async () => {
    // Applying is a separate, user-initiated step; fetching must not have side effects.
    pretendDesktop();
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall});
    download.mockResolvedValue(undefined);

    await fetchUpdate();

    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("carries on when the update feed is unreachable", async () => {
    pretendDesktop();
    check.mockRejectedValue(new Error("network down"));

    await expect(fetchUpdate()).resolves.toEqual({state: "failed"});
  });
});
