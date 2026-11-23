// @vitest-environment jsdom
// VATUSA/OIS#1012: the add-item form names every failed check and refuses an unparseable restriction
// window, driven through the real form. Each refusal breaks one field of an item that otherwise
// saves, and sits beside the save of that valid item.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider, ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const post = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: post, PUT: vi.fn(), DELETE: vi.fn()}}));

import {TmiPackagesSection} from "./tmi-packages";
import type {Me} from "@/lib/auth";
import type {AirportRate, TmiPackage} from "@/lib/events";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const EVENT = 7;
const ITEMS_PATH = "/api/v1/events/{id}/packages/{package_id}/items";

// A real permission tree, not server_admin, which `hasPermission` short-circuits on.
const EDITOR = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Planner",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {events: {plan: ["read", "update"]}},
} as unknown as Me;

const PKG: TmiPackage = {
  id: "p1",
  name: "FNO kickoff",
  status: "draft",
  items: [],
  auto_publish: false,
  updated_at: "2026-10-01T00:00:00Z",
};

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];

// `parseZulu` resolves DD/HHMMz against the current month. Only Date is faked, so React and the
// query cache's setTimeout still run.
beforeEach(() => {
  vi.useFakeTimers({toFake: ["Date"]});
  vi.setSystemTime(new Date("2026-10-12T12:00:00Z"));
  get.mockResolvedValue({data: [PKG], error: undefined, response: {status: 200}});
  post.mockResolvedValue({data: [PKG], error: undefined, response: {status: 200}});
});
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
  get.mockReset();
  post.mockReset();
  vi.useRealTimers();
});

async function mount(me: Me = EDITOR, rates: AirportRate[] = []) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
    },
  });
  qc.setQueryData(["me"], me);
  qc.setQueryData(["event-packages", EVENT], [PKG]);
  qc.setQueryData(["event-rates", EVENT], rates);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <DialogProvider>
            <TmiPackagesSection eventId={EVENT} />
          </DialogProvider>
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
}

const input = (label: string) => {
  const el = document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`);
  if (!el) throw new Error(`no input labelled ${label}`);
  return el;
};
const typeInto = (el: HTMLInputElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event("input", {bubbles: true}));
  });
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const radio = (group: string, label: string) => {
  const el = [...document.querySelectorAll(`[role="radiogroup"][aria-label="${group}"] [role="radio"]`)].find(
    (b) => b.textContent?.trim() === label,
  );
  if (!el) throw new Error(`no ${label} option in ${group}`);
  return el;
};
const addButton = () => {
  const el = [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Add");
  if (!el) throw new Error("no Add button");
  return el;
};
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache and the mutation resolve on later ticks, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

/** Picks the item kind, then fills each field by its aria-label (choosing a kind clears the form). */
async function fill(kind: string, fields: Record<string, string>) {
  await click(radio("Item kind", kind));
  for (const [label, value] of Object.entries(fields)) await typeInto(input(label), value);
}

async function expectRefused(message: string, typed: Record<string, string>) {
  await click(addButton());
  await settle();
  expect(toasts().some((t) => t.includes(message)), `toasts: ${JSON.stringify(toasts())}`).toBe(true);
  expect(post).not.toHaveBeenCalled();
  for (const [label, value] of Object.entries(typed)) expect(input(label).value, label).toBe(value);
}

async function expectSaved(kind: string, payload: Record<string, unknown>) {
  await click(addButton());
  await settle();
  expect(post).toHaveBeenCalledTimes(1);
  expect(post).toHaveBeenCalledWith(ITEMS_PATH, {
    params: {path: {id: EVENT, package_id: "p1"}},
    body: {kind, payload},
  });
  expect(toasts().filter((t) => / is required| must be /.test(t))).toEqual([]);
}

const PROGRAM = {ICAO: "KJFK", AAR: "40", "Minutes in trail": "10", "Miles in trail": "20"};
const RESTRICTION = {
  requesting: "ZNY",
  providing: "ZDC",
  "start DD/HHMMz": "12/1430z",
  "stop DD/HHMMz": "12/1830z",
  "restriction (e.g. 20 MIT)": "20 MIT",
};
const ADVISORY = {
  "Issuing facility, e.g. DCC": "DCC",
  "valid from DD/HHMMz": "12/1430z",
  "valid to DD/HHMMz": "12/1830z",
  "advisory text": "REROUTE",
};

describe("TMI package add-item form (VATUSA/OIS#1012)", () => {
  it("shows the add-item form to a planner without server admin, and hides it without the permission", async () => {
    await mount();
    expect(document.querySelector('[role="radiogroup"][aria-label="Item kind"]')).not.toBeNull();
    expect(addButton()).toBeTruthy();

    for (const {root, host} of roots.splice(0)) {
      act(() => root.unmount());
      host.remove();
    }
    await mount({...EDITOR, permissions: {events: {plan: ["read"]}}} as unknown as Me);
    expect(document.querySelector('[role="radiogroup"][aria-label="Item kind"]')).toBeNull();
  });

  describe("program", () => {
    it.each([
      ["ICAO", "ICAO is required"],
      ["AAR", "AAR is required"],
    ])("refuses a program with no %s, naming the field", async (blank, message) => {
      await mount();
      const typed = {...PROGRAM, [blank]: ""};
      await fill("Program", typed);
      await expectRefused(message, typed);
    });

    it("refuses through the Enter key exactly as through Add", async () => {
      await mount();
      await fill("Program", {...PROGRAM, AAR: ""});
      await act(async () => {
        input("ICAO").dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true}));
      });
      await settle();
      expect(toasts().some((t) => t.includes("AAR is required"))).toBe(true);
      expect(post).not.toHaveBeenCalled();
      expect(input("ICAO").value).toBe("KJFK");
    });

    it("saves a program pre-filled from an airport rate, with MINIT and MIT of 0", async () => {
      const kewr: AirportRate = {
        icao: "KEWR",
        aar: 36,
        adr: 40,
        artcc: "ZNY",
        editable: true,
        source: "override",
        updated_at: "2026-10-01T00:00:00Z",
      };
      await mount(EDITOR, [kewr]);
      const prefill = [...document.querySelectorAll("button")].find((b) => b.title === "Pre-fill KEWR · AAR 36");
      if (!prefill) throw new Error("no KEWR pre-fill button");
      await click(prefill);
      expect(input("ICAO").value).toBe("KEWR");
      expect(input("AAR").value).toBe("36");
      await expectSaved("program", {icao: "KEWR", aar: 36, trail: 0, mit: 0});
    });

    it("saves a valid program", async () => {
      await mount();
      await fill("Program", PROGRAM);
      await expectSaved("program", {icao: "KJFK", aar: 40, trail: 10, mit: 20});
    });
  });

  describe("restriction", () => {
    it.each([
      ["requesting", "", "Requesting is required"],
      ["requesting", "   ", "Requesting is required"],
      ["providing", "", "Providing is required"],
      ["restriction (e.g. 20 MIT)", "", "A restriction is required"],
      ["start DD/HHMMz", "1430z", "Start time must be DD/HHMMz (e.g. 12/1430z)"],
      ["stop DD/HHMMz", "12/2430z", "Stop time must be DD/HHMMz (e.g. 12/1830z)"],
    ])("refuses a restriction whose %s is %j, naming the field and keeping the input", async (field, value, message) => {
      await mount();
      const typed = {...RESTRICTION, [field]: value};
      await fill("Restriction", typed);
      await expectRefused(message, typed);
    });

    it("refuses a structured restriction with no element", async () => {
      await mount();
      await fill("Restriction", {requesting: "ZNY", providing: "ZDC"});
      await click(radio("Restriction entry", "Structured"));
      await expectRefused("An element and a restriction type are required", {requesting: "ZNY", providing: "ZDC"});
    });

    it("saves a valid restriction with its window as ISO start_time and stop_time", async () => {
      await mount();
      await fill("Restriction", RESTRICTION);
      await expectSaved("restriction", {
        requesting: "ZNY",
        providing: "ZDC",
        restriction: "20 MIT",
        structured: undefined,
        start_time: "2026-10-12T14:30:00.000Z",
        stop_time: "2026-10-12T18:30:00.000Z",
      });
    });

    it("saves a restriction with a blank start and stop, omitting both", async () => {
      await mount();
      await fill("Restriction", {...RESTRICTION, "start DD/HHMMz": "", "stop DD/HHMMz": ""});
      await expectSaved("restriction", {
        requesting: "ZNY",
        providing: "ZDC",
        restriction: "20 MIT",
        structured: undefined,
        start_time: undefined,
        stop_time: undefined,
      });
    });

    it("saves a structured restriction carrying the NTML fields", async () => {
      await mount();
      await fill("Restriction", {requesting: "ZNY", providing: "ZDC"});
      await click(radio("Restriction entry", "Structured"));
      const element = document.querySelector<HTMLInputElement>('input[placeholder="JFK / EWR,LGA"]')!;
      await typeInto(element, "JFK");
      await click(addButton());
      await settle();
      expect(post).toHaveBeenCalledTimes(1);
      const payload = post.mock.calls[0][1].body.payload;
      expect(payload.restriction).toBe("");
      expect(payload.structured).toMatchObject({element: "JFK", kind: "MIT"});
    });
  });

  describe("ground stop", () => {
    it("refuses a ground stop with no airport", async () => {
      await mount();
      await fill("Ground stop", {airport: "", "scope (blank=all)": "ZNY"});
      await expectRefused("Airport is required", {"scope (blank=all)": "ZNY"});
    });

    it("saves a ground stop with its scope and until", async () => {
      await mount();
      await fill("Ground stop", {airport: "KDCA", "scope (blank=all)": "ZNY", "until HHMM": "1800"});
      await expectSaved("ground_stop", {airport: "KDCA", scope: "ZNY", until: "1800"});
    });

    it("saves a valid ground stop", async () => {
      await mount();
      await fill("Ground stop", {airport: "KDCA"});
      await expectSaved("ground_stop", {airport: "KDCA", scope: undefined, until: undefined});
    });
  });

  describe("advisory", () => {
    it.each([
      ["Issuing facility, e.g. DCC", "", "Facility is required"],
      ["valid from DD/HHMMz", "", "Valid from must be DD/HHMMz (e.g. 12/1430z)"],
      ["valid from DD/HHMMz", "12/2500z", "Valid from must be DD/HHMMz (e.g. 12/1430z)"],
      ["valid to DD/HHMMz", "", "Valid to must be DD/HHMMz (e.g. 12/1830z)"],
      ["valid to DD/HHMMz", "12/2500z", "Valid to must be DD/HHMMz (e.g. 12/1830z)"],
      ["valid to DD/HHMMz", "12/1430z", "Valid to must be after valid from"],
      ["advisory text", "", "Advisory text is required"],
    ])("refuses an advisory whose %s is %j, naming the field", async (field, value, message) => {
      await mount();
      const typed = {...ADVISORY, [field]: value};
      await fill("Advisory", typed);
      await click(radio("Advisory type", "Reroute"));
      await expectRefused(message, typed);
    });

    it("refuses an advisory with no type chosen", async () => {
      await mount();
      await fill("Advisory", ADVISORY);
      await expectRefused("Advisory type is required", ADVISORY);
    });

    it("saves a valid advisory with ISO valid_from and valid_to", async () => {
      await mount();
      await fill("Advisory", ADVISORY);
      await click(radio("Advisory type", "Reroute"));
      await expectSaved("advisory", {
        facility: "DCC",
        kind: "reroute",
        body: "REROUTE",
        valid_from: "2026-10-12T14:30:00.000Z",
        valid_to: "2026-10-12T18:30:00.000Z",
      });
    });
  });
});
