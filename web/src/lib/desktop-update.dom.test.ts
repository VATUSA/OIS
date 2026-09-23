// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {fetchUpdate, installUpdate} from "./desktop-update";

const check = vi.fn();
const download = vi.fn();
const downloadAndInstall = vi.fn();
const install = vi.fn();
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
  install.mockReset();
  relaunch.mockReset();
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  vi.restoreAllMocks(); // console spies must not leak between tests
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
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall, install});
    download.mockResolvedValue(undefined);

    await expect(fetchUpdate()).resolves.toMatchObject({state: "ready", version: "1.2.3"});
    expect(download).toHaveBeenCalledTimes(1);
  });

  it("does NOT report ready when the signature fails to verify", async () => {
    // What this pins is this module's *reaction* to a rejected package: `failed`, never `ready`
    // (`ready` is what the banner offers a restart for). The rejection itself is the updater plugin's
    // — `download` is mocked here, so this test would pass even if verification were switched off.
    // That the plugin really refuses a tampered package was verified against a genuinely signed
    // build with minisign in the #347 review; keep verification delegated to the plugin.
    pretendDesktop();
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    check.mockResolvedValue({version: "9.9.9", download, downloadAndInstall, install});
    download.mockRejectedValue(new Error("signature verification failed"));

    await expect(fetchUpdate()).resolves.toEqual({state: "failed"});
    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
    // A refused package is a security event, logged as such — not as a routine unreachable feed.
    expect(error).toHaveBeenCalledWith(expect.stringContaining("REJECTED update 9.9.9"), expect.any(Error));
  });

  it("logs an unreachable feed as a warning, not as a rejected package", async () => {
    pretendDesktop();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    check.mockRejectedValue(new Error("network down"));

    await expect(fetchUpdate()).resolves.toEqual({state: "failed"});
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("could not check"), expect.any(Error));
    expect(error).not.toHaveBeenCalled();
  });

  it("never restarts the app on its own, even for a good update", async () => {
    // Applying is a separate, user-initiated step; fetching must not have side effects.
    pretendDesktop();
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall, install});
    download.mockResolvedValue(undefined);

    await fetchUpdate();

    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(install).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("carries on when the update feed is unreachable", async () => {
    pretendDesktop();
    check.mockRejectedValue(new Error("network down"));

    await expect(fetchUpdate()).resolves.toEqual({state: "failed"});
  });
});

describe("installUpdate", () => {
  it("applies the package that was verified, without asking the feed again", async () => {
    // The whole point of holding the staged handle: re-`check()`ing would download the package a
    // second time and apply whatever the feed serves *then*, not the thing the banner offered.
    pretendDesktop();
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall, install});
    download.mockResolvedValue(undefined);
    install.mockResolvedValue(undefined);

    const status = await fetchUpdate();
    expect(status.state).toBe("ready");
    check.mockClear();

    if (status.state !== "ready") throw new Error("expected a staged update");
    await installUpdate(status.staged);

    expect(install).toHaveBeenCalledTimes(1);
    expect(check).not.toHaveBeenCalled();
    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).toHaveBeenCalledTimes(1);
  });

  it("propagates a failure to apply rather than swallowing it", async () => {
    pretendDesktop();
    check.mockResolvedValue({version: "1.2.3", download, downloadAndInstall, install});
    download.mockResolvedValue(undefined);
    install.mockRejectedValue(new Error("could not apply"));

    const status = await fetchUpdate();
    if (status.state !== "ready") throw new Error("expected a staged update");

    await expect(installUpdate(status.staged)).rejects.toThrow(/could not apply/);
    expect(relaunch).not.toHaveBeenCalled();
  });
});
