// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * Sign-in has to invalidate `["me"]`, or the desktop app never leaves the landing page (#428).
 *
 * `login()` resolves in place on desktop — no navigation, no event — and `fetchMe` has already cached
 * the 401 as `null` for `staleTime`, so the only thing that can re-render the app signed in is that
 * invalidation. Two call sites had drifted onto the bare `login()` and each showed the bug.
 *
 * The mocks stop at the Tauri boundary deliberately: the real `login()` and the real `useLogin()` run,
 * so reverting the button to `onClick={login}` turns this red rather than quietly still passing.
 */
const desktopLogin = vi.hoisted(() => vi.fn(async () => undefined as unknown));
vi.mock("@/lib/desktop-auth", () => ({
  desktopLogin,
  desktopLogout: vi.fn(),
}));
vi.mock("@/lib/platform", () => ({
  isTauri: () => true,
  can: () => true,
  MAIN_WINDOW_LABEL: "main",
}));

import {SignInButton} from "./sign-in-button";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLElement | undefined;
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
});
beforeEach(() => {
  desktopLogin.mockReset();
  desktopLogin.mockResolvedValue(undefined);
});

/** A client with `["me"]` already seeded to "signed out", which is the state the bug lives in. */
function signedOutClient() {
  const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
  qc.setQueryData(["me"], null);
  return qc;
}

async function render(qc: QueryClient) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => {
    root!.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <SignInButton />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host.querySelector("button")!;
}

describe("SignInButton", () => {
  it("invalidates the cached signed-out ['me'] so the app re-renders signed in", async () => {
    const qc = signedOutClient();
    expect(qc.getQueryState(["me"])?.isInvalidated).toBe(false);

    const button = await render(qc);
    await act(async () => button.click());
    // TanStack notifies the cache outside React's scheduler, so let a macrotask run.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(desktopLogin).toHaveBeenCalledTimes(1);
    expect(qc.getQueryState(["me"])?.isInvalidated).toBe(true);
  });

  it("disables itself while a sign-in is in flight, so a second click can't wedge the listener", async () => {
    // The loopback listener is a fixed port held for LOGIN_TIMEOUT, so a double-click used to hit
    // EADDRINUSE and leave the user stuck for five minutes.
    let release: () => void = () => undefined;
    desktopLogin.mockImplementation(
      () => new Promise<undefined>((resolve) => (release = () => resolve(undefined))),
    );

    const button = await render(signedOutClient());
    await act(async () => button.click());
    // Same reason as above: the mutation's pending state is notified outside React's scheduler.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(button.disabled).toBe(true);
    expect(button.textContent).toContain("Signing in");

    await act(async () => {
      release();
    });
    expect(desktopLogin).toHaveBeenCalledTimes(1);
  });

  it("surfaces a failed sign-in instead of failing silently", async () => {
    desktopLogin.mockRejectedValue(new Error("could not listen on 127.0.0.1:8765"));

    const button = await render(signedOutClient());
    await act(async () => button.click());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(document.body.textContent).toContain("Sign-in failed");
    expect(document.body.textContent).toContain("could not listen on 127.0.0.1:8765");
  });
});
