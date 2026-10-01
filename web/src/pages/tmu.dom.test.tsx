// @vitest-environment jsdom
//
// VATUSA/OIS#506 — the TMU tab list's permission gates.
//
// Each tab in `TmuPage` is gated on a read permission, and none of those gates had a test:
// replacing any one with `true` left the whole suite green. That matters because this is the
// mechanism every TMU feature's "a non-holder sees no control" acceptance criterion points at —
// #460's AC 3 cites it by name — so the thing several ACs are satisfied *by* was unchecked.
//
// These assert on **which tab's content renders**, not on the tab bar, for a structural reason:
// `TmuPage` only renders `<Tabs>` when `tabs.length > 1`, so a user holding exactly one permission
// has no tab bar to read. Content is observable in every case, and it is also the thing that
// actually matters — a tab you cannot reach is the point of the gate.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

import type {Me} from "@/lib/auth";

const search = vi.hoisted(() => ({current: {} as {tab?: string; facility?: string}}));
const me = vi.hoisted(() => ({current: null as unknown}));
const tabItems = vi.hoisted(() => ({current: null as null | {value: string; label: string}[]}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => search.current,
  useNavigate: () => () => undefined,
}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: me.current})}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => undefined}));

// `Tabs` is captured rather than rendered so the *whole* permitted list can be asserted, which is
// what catches a tab added with no gate at all. `EmptyState` keeps its text, since the no-access
// case is asserted through it.
vi.mock("@ois/ui", () => ({
  Tabs: (props: {items: {value: string; label: string}[]}) => {
    tabItems.current = props.items;
    return null;
  },
  EmptyState: ({children}: {children?: unknown}) => <div data-testid="empty">{children as never}</div>,
}));

// Each tab's content is a marker, so "which tab is open" is a DOM query rather than a full page
// render with its own data requirements.
vi.mock("@/pages/tmu/programs", () => ({ProgramsTab: () => <div data-tab="programs" />}));
vi.mock("@/pages/tmu/restrictions", () => ({RestrictionsTab: () => <div data-tab="restrictions" />}));
vi.mock("@/pages/tmu/ground-stops", () => ({GroundStopsTab: () => <div data-tab="ground-stops" />}));
vi.mock("@/pages/tmu/gdp", () => ({GdpTab: () => <div data-tab="gdp" />}));
vi.mock("@/pages/tmu/rate-calc", () => ({RateCalculatorTab: () => <div data-tab="rate-calculator" />}));

import {TmuPage} from "./tmu";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

/**
 * A user holding exactly `perms`, built into the nested shape `hasPermission` walks.
 *
 * Deliberately **not** mocking `hasPermission`: it is part of what #506 says is untested, so a test
 * that stubbed it would assert the gates call *something* rather than that they gate correctly.
 */
function user(perms: string[]): Me {
  const permissions: Record<string, unknown> = {};
  for (const p of perms) {
    const parts = p.split(".");
    const action = parts.pop()!;
    let node = permissions;
    for (const seg of parts) {
      node[seg] ??= {};
      node = node[seg] as Record<string, unknown>;
    }
    // The leaf is an action array; `hasPermission` requires `Array.isArray`.
    const leaf = parts.at(-1)!;
    const parent = parts.slice(0, -1).reduce<Record<string, unknown>>(
      (acc, seg) => acc[seg] as Record<string, unknown>,
      permissions,
    );
    const existing = parent[leaf];
    parent[leaf] = Array.isArray(existing) ? [...existing, action] : [action];
  }
  return {
    cid: 1234567,
    display_name: "Test Controller",
    email: "test@example.com",
    id: "u-1",
    permissions: permissions as Me["permissions"],
    role_names: [],
    server_admin: false,
    tmu_national: false,
  };
}

/**
 * The mounted root, torn down in `afterEach`.
 *
 * Deliberately **not** unmounted inside `render` the way `fca/index.dom.test.tsx` does: that test
 * captures props into a ref, so the DOM being gone does not matter. These assertions query the DOM,
 * and unmounting first empties it — every one of them passes vacuously or fails confusingly.
 */
let mounted: ReturnType<typeof createRoot> | null = null;

function render(perms: string[], tab?: string): HTMLElement {
  me.current = user(perms);
  search.current = tab ? {tab} : {};
  tabItems.current = null;
  const host = document.createElement("div");
  mounted = createRoot(host);
  act(() => mounted!.render(<TmuPage />));
  return host;
}

/** Every tab, and the permission that admits it. `tmu.program.read` admits two. */
const GATES: [permission: string, tab: string, label: string][] = [
  ["tmu.program.read", "programs", "Programs"],
  ["tmu.tmi.read", "restrictions", "Restrictions"],
  ["tmu.groundstop.read", "ground-stops", "Ground stops"],
  ["tmu.gdp.read", "gdp", "Ground delay"],
  ["tmu.program.read", "rate-calculator", "Rate calculator"],
];

const ALL = [...new Set(GATES.map(([p]) => p))];

describe("TMU tab permission gates (VATUSA/OIS#506)", () => {
  beforeEach(() => {
    tabItems.current = null;
  });

  afterEach(() => {
    if (mounted) {
      const root = mounted;
      act(() => root.unmount());
      mounted = null;
    }
  });

  // AC 2 — and the half that catches an *inverted* gate, which a hidden-only test would miss.
  it.each(GATES)("admits %s's tab to a holder", (permission, tab) => {
    const host = render([permission], tab);
    expect(host.querySelector(`[data-tab="${tab}"]`)).not.toBeNull();
  });

  // AC 1 — a holder of every *other* permission still cannot reach this tab, even by asking for it
  // in `?tab=`. Granting the rest is what makes this fail on a removed gate rather than merely on
  // an empty page.
  it.each(GATES)("keeps %s's tab from a non-holder who asks for it", (permission, tab) => {
    const others = ALL.filter((p) => p !== permission);
    const host = render(others, tab);
    expect(host.querySelector(`[data-tab="${tab}"]`)).toBeNull();
  });

  // AC 3, structurally: if any tab were added without a gate, `tabs` could not be empty and this
  // would render content instead of the no-access state. It needs no update when a tab is added.
  it("shows no tab at all to a user with no TMU permissions", () => {
    const host = render([]);
    expect(host.querySelector("[data-testid='empty']")?.textContent).toContain(
      "traffic-management access",
    );
    expect(host.querySelector("[data-tab]")).toBeNull();
  });

  // AC 3, the other half: the full permitted list is pinned, so adding a sixth tab fails here and
  // whoever adds it has to extend `GATES` — which is the review signal #506 asked for.
  it("offers exactly the known tabs to a holder of everything", () => {
    render(ALL);
    expect(tabItems.current?.map((t) => t.value)).toEqual(GATES.map(([, tab]) => tab));
    expect(tabItems.current?.map((t) => t.label)).toEqual(GATES.map(([, , label]) => label));
  });
});
