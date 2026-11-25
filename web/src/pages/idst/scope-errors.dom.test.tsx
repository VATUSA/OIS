// @vitest-environment jsdom
//
// VATUSA/OIS#1009: a failed IDST scope load shows an error with Retry, a failed scope save raises a
// toast, and an invalid airport keeps its text under an inline message. Each failure sits beside a
// control that loads or saves normally. The API client is mocked at its boundary because a failed
// GET cannot be seeded through the query cache.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const api = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn(), POST: vi.fn(), DELETE: vi.fn() }));
vi.mock("@/lib/api", () => ({ ois: api, API_BASE: "", DOCS_URL: "" }));

import type {Me} from "@/lib/auth";
import {IdstPage} from "./index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const PREFS_PATH = "/api/v1/me/preferences/{namespace}";
const LOAD_ERROR = "Couldn't load your IDST scope.";
const SAVE_ERROR = "Couldn't save your IDST scope";
const AIRPORT_ERROR = "Airport must be 3 or 4 letters or digits";
const EMPTY_PROMPT = "Set your scope to see FCA-metered ground departures.";

const OK = (data?: unknown) => ({ data, error: undefined, response: { ok: true, status: 200 } });
const FAIL_WITH_BODY = { data: undefined, error: { error: "internal" }, response: { ok: false, status: 500 } };
const FAIL_BODYLESS = { data: undefined, error: undefined, response: { ok: false, status: 502 } };

type Init = { params?: { path?: { namespace?: string } }; body?: unknown };
const isIdstPrefs = (path: string, init?: Init) => path === PREFS_PATH && init?.params?.path?.namespace === "idst";
const prefGets = () => api.GET.mock.calls.filter(([p, i]) => isIdstPrefs(p, i));
const prefPuts = () => api.PUT.mock.calls.filter(([p, i]) => isIdstPrefs(p, i));

/** Answers the idst preferences GET with each of `answers` in turn (the last repeats); everything else ok. */
function prefsGetAnswers(...answers: unknown[]) {
  let n = 0;
  api.GET.mockImplementation((path: string, init?: Init) => {
    if (isIdstPrefs(path, init)) return Promise.resolve(answers[Math.min(n++, answers.length - 1)]);
    if (path === "/api/v1/flow/idst") return Promise.resolve(OK({ unscheduled: [], released: [], metered_count: 0 }));
    return Promise.resolve(OK());
  });
}

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.PUT.mockResolvedValue(OK());
});

const flush = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });

async function mount() {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
      mutations: { retry: false },
    },
  });
  qc.setQueryData(["me"], { server_admin: false, permissions: {} } as unknown as Me);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <IdstPage />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  await vi.waitFor(async () => {
    await flush();
    expect(qc.getQueryState(["preferences", "idst"])?.fetchStatus).toBe("idle");
    expect(qc.getQueryState(["preferences", "idst"])?.status).not.toBe("pending");
  });
  return qc;
}

const text = () => document.body.textContent ?? "";
const input = () => document.querySelector<HTMLInputElement>("#idst-airport")!;
const alertText = () => document.querySelector('[role="alert"]')?.textContent ?? null;
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === label);
const chips = () =>
  [...document.querySelectorAll("button, span, div")]
    .map((e) => e.textContent ?? "")
    .filter((t) => /^Airport\s*KDCA/.test(t.trim()));

async function type(value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function clickAdd() {
  await act(async () => button("Add")!.click());
  await flush();
  await flush();
}
async function pressEnter() {
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  await flush();
}

describe("IdstPage scope load errors", () => {
  it("shows the load error and a Retry button when the scope GET fails", async () => {
    prefsGetAnswers(FAIL_WITH_BODY);
    await mount();
    expect(text()).toContain(LOAD_ERROR);
    expect(button("Retry")).toBeDefined();
    expect(text()).not.toContain(EMPTY_PROMPT);
  });

  it("shows the set-your-scope prompt and no error when the GET is ok", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    expect(text()).toContain(EMPTY_PROMPT);
    expect(text()).not.toContain(LOAD_ERROR);
    expect(button("Retry")).toBeUndefined();
  });

  it("clears the load error once Retry refetches successfully", async () => {
    prefsGetAnswers(FAIL_WITH_BODY, OK({ airports: ["KDCA"], tracons: [], artccs: [] }));
    await mount();
    expect(text()).toContain(LOAD_ERROR);

    await act(async () => button("Retry")!.click());
    await vi.waitFor(async () => {
      await flush();
      expect(text()).not.toContain(LOAD_ERROR);
    });
    expect(prefGets()).toHaveLength(2);
    expect(chips().length).toBeGreaterThan(0);
  });

  it("keeps the load error when Retry fails again", async () => {
    prefsGetAnswers(FAIL_WITH_BODY);
    await mount();
    await act(async () => button("Retry")!.click());
    await vi.waitFor(async () => {
      await flush();
      expect(prefGets()).toHaveLength(2);
    });
    await flush();
    expect(text()).toContain(LOAD_ERROR);
  });

  it("sends no PUT when an airport is added after a failed load, and does after an ok one", async () => {
    prefsGetAnswers(FAIL_WITH_BODY);
    await mount();
    await type("KDCA");
    await clickAdd();
    expect(prefPuts()).toHaveLength(0);

    act(() => roots.splice(0).forEach(({ root, host }) => (root.unmount(), host.remove())));
    prefsGetAnswers(OK({}));
    await mount();
    await type("KDCA");
    await clickAdd();
    expect(prefPuts()).toHaveLength(1);
  });
});

describe("IdstPage scope save errors", () => {
  it("toasts and adds no chip when the scope PUT fails with an error body", async () => {
    prefsGetAnswers(OK({}));
    api.PUT.mockResolvedValue(FAIL_WITH_BODY);
    await mount();
    await type("KDCA");
    await clickAdd();
    expect(prefPuts()).toHaveLength(1);
    expect(text()).toContain(SAVE_ERROR);
    expect(chips()).toHaveLength(0);
  });

  it("toasts when the scope PUT fails with no error body", async () => {
    prefsGetAnswers(OK({}));
    api.PUT.mockResolvedValue(FAIL_BODYLESS);
    await mount();
    await type("KDCA");
    await clickAdd();
    expect(prefPuts()).toHaveLength(1);
    expect(text()).toContain(SAVE_ERROR);
    expect(chips()).toHaveLength(0);
  });

  it("adds the chip, sends airports [\"KDCA\"] and shows no toast when the PUT is ok", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type("KDCA");
    await clickAdd();
    expect(prefPuts()).toHaveLength(1);
    expect(prefPuts()[0][1].body).toEqual({ airports: ["KDCA"], tracons: [], artccs: [] });
    expect(chips().length).toBeGreaterThan(0);
    expect(text()).not.toContain(SAVE_ERROR);
  });

  it("keeps the chip and toasts when removing it fails to save", async () => {
    prefsGetAnswers(OK({ airports: ["KDCA"], tracons: [], artccs: [] }));
    api.PUT.mockResolvedValue(FAIL_WITH_BODY);
    await mount();
    expect(chips().length).toBeGreaterThan(0);

    const clear = document.querySelector<HTMLButtonElement>('button[aria-label="Clear filter"]'); // the only chip is KDCA
    expect(clear).not.toBeNull();
    await act(async () => clear!.click());
    await flush();
    await flush();
    expect(prefPuts()).toHaveLength(1);
    expect(prefPuts()[0][1].body).toEqual({ airports: [], tracons: [], artccs: [] });
    expect(text()).toContain(SAVE_ERROR);
    expect(chips().length).toBeGreaterThan(0);
  });
});

describe("IdstPage airport input validation", () => {
  it("shows the inline message, keeps \"KD!\" and sends no PUT", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type("KD!");
    await clickAdd();
    expect(alertText()).toBe(AIRPORT_ERROR);
    expect(input().value).toBe("KD!");
    expect(input().getAttribute("aria-invalid")).toBe("true");
    const describedBy = input().getAttribute("aria-describedby");
    expect(describedBy && document.getElementById(describedBy)?.textContent).toBe(AIRPORT_ERROR);
    expect(prefPuts()).toHaveLength(0);
  });

  it("rejects a 2-character and a 5-character airport inline", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    for (const v of ["KD", "KDCAX"]) {
      await type(v);
      await clickAdd();
      expect(alertText(), v).toBe(AIRPORT_ERROR);
      expect(input().value, v).toBe(v);
    }
    expect(prefPuts()).toHaveLength(0);
  });

  it("accepts a 3-character airport", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type("dca");
    await clickAdd();
    expect(alertText()).toBeNull();
    expect(input().value).toBe("");
    expect(prefPuts()[0][1].body).toEqual({ airports: ["DCA"], tracons: [], artccs: [] });
  });

  it("does nothing for a blank or whitespace-only airport", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    for (const v of ["", "   "]) {
      await type(v);
      await clickAdd();
      expect(alertText(), JSON.stringify(v)).toBeNull();
    }
    expect(prefPuts()).toHaveLength(0);
  });

  it("shows the inline message when Enter submits an invalid airport", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type("KD!");
    await pressEnter();
    expect(alertText()).toBe(AIRPORT_ERROR);
    expect(input().value).toBe("KD!");
    expect(prefPuts()).toHaveLength(0);
  });

  it("saves \" kdca \" as [\"KDCA\"], clears the input and shows no message", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type(" kdca ");
    await clickAdd();
    expect(alertText()).toBeNull();
    expect(input().value).toBe("");
    expect(prefPuts()).toHaveLength(1);
    expect(prefPuts()[0][1].body).toEqual({ airports: ["KDCA"], tracons: [], artccs: [] });
  });

  it("clears the inline message when the user edits the input", async () => {
    prefsGetAnswers(OK({}));
    await mount();
    await type("KD!");
    await clickAdd();
    expect(alertText()).toBe(AIRPORT_ERROR);
    await type("KD");
    expect(alertText()).toBeNull();
    expect(input().getAttribute("aria-invalid")).toBeNull();
  });
});
