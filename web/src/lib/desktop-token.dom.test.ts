// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const invokeDesktop = vi.fn();
let onDesktop = false;

vi.mock("@/lib/platform", () => ({
  isTauri: () => onDesktop,
  invokeDesktop: (...args: unknown[]) => invokeDesktop(...args),
}));

// The cache is module state, so each case needs a fresh copy of the module.
async function freshModule() {
  vi.resetModules();
  return import("./desktop-token");
}

beforeEach(() => {
  invokeDesktop.mockReset();
  onDesktop = false;
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("getDesktopToken", () => {
  it("never touches the keychain on the web build", async () => {
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
    expect(invokeDesktop).not.toHaveBeenCalled();
  });

  it("reads the keychain on desktop and caches the result", async () => {
    onDesktop = true;
    invokeDesktop.mockResolvedValue("ois_dsk_abc");
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBe("ois_dsk_abc");
    await expect(getDesktopToken()).resolves.toBe("ois_dsk_abc");
    expect(invokeDesktop).toHaveBeenCalledTimes(1);
    expect(invokeDesktop).toHaveBeenCalledWith("get_token");
  });

  it("shares one keychain read between concurrent callers", async () => {
    // On a cold start every query fires at once; without the shared promise each would read.
    onDesktop = true;
    invokeDesktop.mockResolvedValue("ois_dsk_abc");
    const {getDesktopToken} = await freshModule();

    const results = await Promise.all([getDesktopToken(), getDesktopToken(), getDesktopToken()]);

    expect(results).toEqual(["ois_dsk_abc", "ois_dsk_abc", "ois_dsk_abc"]);
    expect(invokeDesktop).toHaveBeenCalledTimes(1);
  });

  it("reports signed out when the keychain holds nothing", async () => {
    onDesktop = true;
    invokeDesktop.mockResolvedValue(null);
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
  });

  it("reports signed out rather than throwing when the keychain is unavailable", async () => {
    // A locked or broken keychain must not take the whole app down; it just means no bearer.
    onDesktop = true;
    invokeDesktop.mockRejectedValue(new Error("keychain unavailable"));
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
  });

  /**
   * The loop this issue is about (VATUSA/OIS#535 AC4). On macOS a keychain read is ACL-gated and can
   * put up a login-password prompt; dismissing it rejects `get_token`. The token is resolved per API
   * request and `useMe` retries every 5s on error, so a read that leaves no trace re-prompts for as
   * long as the app is open. One failed read must mean one failed read.
   */
  it("a rejected read is not retried on the next request", async () => {
    onDesktop = true;
    invokeDesktop.mockRejectedValue(new Error("user dismissed the keychain prompt"));
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
    await expect(getDesktopToken()).resolves.toBeUndefined();
    await expect(getDesktopToken()).resolves.toBeUndefined();

    expect(invokeDesktop).toHaveBeenCalledTimes(1);
  });

  it("an empty keychain is not re-read either", async () => {
    // Signed out is a settled answer: only `setDesktopToken` can change it, so re-reading would be
    // one more ACL-gated keychain hit for a result that cannot have moved.
    onDesktop = true;
    invokeDesktop.mockResolvedValue(null);
    const {getDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
    await expect(getDesktopToken()).resolves.toBeUndefined();

    expect(invokeDesktop).toHaveBeenCalledTimes(1);
  });
});

describe("setDesktopToken", () => {
  it("makes the new token available without re-reading the keychain", async () => {
    onDesktop = true;
    const {getDesktopToken, setDesktopToken} = await freshModule();

    setDesktopToken("ois_dsk_rotated");

    await expect(getDesktopToken()).resolves.toBe("ois_dsk_rotated");
    expect(invokeDesktop).not.toHaveBeenCalled();
  });

  it("clearing it sends the next caller back to the keychain", async () => {
    onDesktop = true;
    invokeDesktop.mockResolvedValue(null);
    const {getDesktopToken, setDesktopToken} = await freshModule();

    setDesktopToken("ois_dsk_old");
    setDesktopToken(undefined);

    await expect(getDesktopToken()).resolves.toBeUndefined();
    expect(invokeDesktop).toHaveBeenCalledTimes(1);
  });

  /**
   * The recovery path for AC4's sticky flag. A failed read leaves the user on the sign-in screen;
   * signing in calls `setDesktopToken` (`desktop-auth.ts:22`). If that did not re-open reads, a
   * single dismissed prompt would wedge the app as signed-out until it was restarted — the flag
   * would have traded a prompt loop for a dead end.
   */
  it("re-opens reads after a failure, so signing in recovers", async () => {
    onDesktop = true;
    invokeDesktop.mockRejectedValueOnce(new Error("user dismissed the keychain prompt"));
    const {getDesktopToken, setDesktopToken} = await freshModule();

    await expect(getDesktopToken()).resolves.toBeUndefined();
    expect(invokeDesktop).toHaveBeenCalledTimes(1);

    setDesktopToken("ois_dsk_after_sign_in");
    await expect(getDesktopToken()).resolves.toBe("ois_dsk_after_sign_in");

    // And a later logout leaves the keychain readable again rather than permanently settled.
    setDesktopToken(undefined);
    invokeDesktop.mockResolvedValue("ois_dsk_from_keychain");
    await expect(getDesktopToken()).resolves.toBe("ois_dsk_from_keychain");
    expect(invokeDesktop).toHaveBeenCalledTimes(2);
  });

  // A keychain read started just before logout resolves after it. Writing its result into the cache
  // would put the revoked token back and keep sending it (VATUSA/OIS#346 review).
  it("a keychain read that finishes after logout does not restore the token", async () => {
    onDesktop = true;
    let finishRead: (token: string) => void = () => {};
    invokeDesktop.mockReturnValueOnce(new Promise<string>((resolve) => (finishRead = resolve)));
    const {getDesktopToken, setDesktopToken} = await freshModule();

    const inFlight = getDesktopToken();
    setDesktopToken(undefined); // logout, while the read is still pending
    finishRead("ois_dsk_revoked");

    await expect(inFlight).resolves.toBeUndefined();
    invokeDesktop.mockResolvedValue(null);
    await expect(getDesktopToken()).resolves.toBeUndefined();
  });
});
