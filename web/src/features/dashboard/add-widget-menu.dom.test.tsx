// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {AddWidgetMenu} from "./AddWidgetMenu";
import type {AtcWidget, TableWidget, Widget} from "./types";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // Radix's menu primitives call pointer-capture APIs jsdom doesn't implement.
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.scrollIntoView ??= () => {};
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

/** A ZDC controller. Not `server_admin` — that would let the menu render for the wrong reason. */
const zdcController = {
  id: "u1",
  cid: 1,
  email: "a@b.c",
  display_name: "Tester",
  rating: null,
  server_admin: false,
  role_names: [],
  tmu_national: false,
  permissions: {},
  vatusa: { home_facility: "ZDC", visits: [] },
};

const nationalController = { ...zdcController, tmu_national: true };

/**
 * The two ATC menu items, spelled with their badges because the labels alone are identical.
 *
 * Written as the rendered `textContent` — label then badge, no separator — which is what `select`
 * matches against.
 */
const NAS_ATC_ITEM = "Online ATC positionsNAS";
const FACILITY_ATC_ITEM = "Online ATC positionsatc";

/** Mounts the real menu against a seeded cache and opens it. */
async function openMenu(me: unknown) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchInterval: false,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        staleTime: Infinity,
      },
    },
  });
  qc.setQueryData(["me"], me);

  const added: Widget[] = [];
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <DialogProvider>
          <AddWidgetMenu onAdd={(w) => added.push(w)} />
        </DialogProvider>
      </QueryClientProvider>,
    );
  });

  const trigger = host.querySelector("button");
  if (!trigger) throw new Error("no trigger button rendered");
  await act(async () => {
    // Radix opens the menu on pointerdown, which jsdom does not synthesise from a click.
    trigger.dispatchEvent(
      new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0 }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  // The content renders in a portal on document.body, not inside `host`.
  /**
   * Click the menu item whose text starts with `label`.
   *
   * **The two ATC items share a label.** Both the facility one and the national one read
   * `Online ATC positions`, separated only by their trailing badge — `atc` and `NAS`. So a
   * `startsWith` match on the plain label silently takes the *facility* item, and a test meaning to
   * exercise the NAS path would assert against the wrong one. Pass the badge too (see
   * `NAS_ATC_ITEM` / `FACILITY_ATC_ITEM`) rather than the bare label.
   */
  const select = (label: string) => {
    const item = [...document.querySelectorAll('[role="menuitem"]')].find((el) =>
      (el.textContent ?? "").startsWith(label),
    );
    if (!item) throw new Error(`no menu item labelled ${label}`);
    act(() => {
      item.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true }));
      item.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, cancelable: true }));
      item.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });
  };

  return { text: document.body.textContent ?? "", added, select };
}

describe("AddWidgetMenu national scope (VATUSA/OIS#474)", () => {
  it("offers the NAS section to a national reader", async () => {
    const { text } = await openMenu(nationalController);
    expect(text).toContain("National (NAS)");
  });

  it("does not offer it to a facility-scoped controller", async () => {
    const { text } = await openMenu(zdcController);
    // The facility ATC item still exists, so assert on the national section specifically.
    expect(text).toContain("Online ATC positions");
    expect(text).not.toContain("National (NAS)");
  });

  // --- The national data sources (VATUSA/OIS#475) ---

  it("offers the NAS sources to a national reader", async () => {
    const { text } = await openMenu(nationalController);
    expect(text).toContain("NAS — demand vs capacity");
    expect(text).toContain("NAS — FCA pressure");
  });

  it("hides the NAS sources from a facility-scoped controller", async () => {
    const { text } = await openMenu(zdcController);
    expect(text).not.toContain("NAS — demand vs capacity");
    expect(text).not.toContain("NAS — FCA pressure");
  });

  // --- What the menu actually hands over (VATUSA/OIS#497) ---
  //
  // The tests above pin the NAS item's *visibility*, and `atc-widget.dom.test.tsx` pins that a
  // nationally-scoped view renders every facility. The link between them — the payload the menu
  // emits — had no test either side of it: changing the National (NAS) item to
  // `facility: { kind: "artcc", id: "ZDC" }` left all 9 dashboard tests green while that item
  // silently added a single-facility board titled `ZDC · ATC`.
  //
  // `openMenu` already collected `added` and never asserted on it. These assert on it.

  it("adds a nationally-scoped ATC widget from the NAS item", async () => {
    const { added, select } = await openMenu(nationalController);
    select(NAS_ATC_ITEM);

    expect(added).toHaveLength(1);
    expect(added[0]).toMatchObject({ kind: "atc", facility: { kind: "national" } });
    // `NationalRef` carries no id — an `artcc`/`tracon` scope would, so this fails on a swap to one
    // even if `kind` were somehow still reported as national.
    expect(added[0]).not.toHaveProperty("facility.id");
  });

  it("does not emit a national scope from the facility ATC item", async () => {
    const { added, select } = await openMenu(nationalController);
    select(FACILITY_ATC_ITEM);

    // The facility item defers to the facility picker rather than adding anything itself, so
    // nothing is emitted yet. That is the assertion: were it changed to add directly, it could only
    // do so with a scope it had not been given — which is the regression this guards.
    expect(added).toHaveLength(0);
    expect(added.some((w) => w.kind === "atc" && w.facility.kind === "national")).toBe(false);
  });

  it("offers the NAS item only to a national reader, by payload and not only by label", async () => {
    // The complement of the two visibility tests above, at the payload level: a facility-scoped
    // controller has no item that can produce a national widget at all.
    const { added, select } = await openMenu(zdcController);
    expect(() => select(NAS_ATC_ITEM)).toThrow();
    expect(added).toHaveLength(0);
  });

  // #497's optional third criterion, and the only one of the three that `vitest` alone cannot carry:
  // it is `pnpm typecheck` that enforces it. `types.ts` keeps `AtcWidget.facility` on `ScopeRef`
  // while the table and chart widgets keep `params.facility` on `FacilityRef`, so a national scope
  // is *unrepresentable* on a widget whose data source is per-facility — #474's AC 1, built
  // correctly. Nothing pinned it, so widening one of those to `ScopeRef` later would pass every
  // runtime gate silently: the widget would fan out one request per facility in the country, every
  // poll, exactly as `types.ts:90-94` warns.
  //
  // If that widening happens, the `@ts-expect-error` below has nothing left to suppress and `tsc`
  // fails with TS2578 — which is the signal. It does not assert anything at runtime.
  it("keeps a national scope unrepresentable on a fan-out-backed widget", () => {
    const atcScope: AtcWidget["facility"] = { kind: "national" };
    expect(atcScope.kind).toBe("national");

    // @ts-expect-error a per-facility widget's scope must not accept a national ref (#474 AC 1)
    const tableScope: NonNullable<TableWidget["params"]>["facility"] = { kind: "national" };
    expect(tableScope).toBeDefined();
  });

  it("keeps the NAS sources out of the general Tables and Charts lists", async () => {
    // They live only under the gated National section. If they leaked into the general lists, a
    // facility controller would see them — which the test above would catch — but a national
    // reader would also see each one twice, which it would not.
    const { text } = await openMenu(nationalController);
    expect(text.match(/NAS — demand vs capacity/g) ?? []).toHaveLength(1);
    expect(text.match(/NAS — FCA pressure/g) ?? []).toHaveLength(1);
  });

  it("adds the demand table already ranked by exceedance", async () => {
    // A national table is only useful ranked — the user must not have to sort it themselves to see
    // what is overloaded.
    const { added, select } = await openMenu(nationalController);
    select("NAS — demand vs capacity");
    expect(added).toHaveLength(1);
    expect(added[0]).toMatchObject({
      kind: "table",
      source: "nas-demand",
      sort: [{ id: "exceedance", desc: true }],
    });
  });
});
