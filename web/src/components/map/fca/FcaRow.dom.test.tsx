// @vitest-environment jsdom
//
// VATUSA/OIS#736. The plain FCA routes refuse an event FCA's writes to anyone without
// `events.plan.update` (409 `event_fca` once it is published). The live map's row must not offer a
// toggle, Edit or Delete that can only fail — every rostered controller holds `flow.fca.*` (#730).
import {act} from "react";
import {createRoot} from "react-dom/client";
import {DndContext} from "@dnd-kit/core";
import {SortableContext} from "@dnd-kit/sortable";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {Me} from "@/lib/auth";
import type {Fca} from "@/lib/fca";
import {FcaRow} from "./FcaRow";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const FLOW = { flow: { fca: ["read", "update", "delete"] } };
/** A rostered controller: `flow.fca.*`, no event planning. */
const CONTROLLER = { server_admin: false, permissions: FLOW } as unknown as Me;
/** Sees the event builder (`events.plan.read`) but can't plan: still refused by the server. */
const PLAN_READER = {
  server_admin: false,
  permissions: { ...FLOW, events: { plan: ["read"] } },
} as unknown as Me;
const PLANNER = {
  server_admin: false,
  permissions: { ...FLOW, events: { plan: ["read", "update"] } },
} as unknown as Me;

const fca = (eventId: number | null) =>
  ({
    id: eventId == null ? "plain" : "ev-published",
    name: eventId == null ? "PLAIN" : "EVENT",
    color: "#efc14d",
    artcc: "ZDC",
    enabled: true,
    event_id: eventId,
    event_status: eventId == null ? null : "published",
  }) as unknown as Fca;

/** Mounts one row with `me` seeded in the real query cache; `canEdit`/`canDelete` as the live map
 *  computes them for a `flow.fca.*` holder. */
async function mount(me: Me, row: Fca) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(["me"], me);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <DndContext>
            <SortableContext items={[row.id]}>
              <ul>
                <FcaRow
                  fca={row}
                  selected={false}
                  count={0}
                  canEdit
                  canDelete
                  onSelect={() => {}}
                  onToggleEnabled={() => {}}
                  onEdit={() => {}}
                  onDelete={() => {}}
                />
              </ul>
            </SortableContext>
          </DndContext>
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

/** Which write controls the row offers. */
const controls = (host: HTMLElement) => ({
  toggle: host.querySelector('[aria-label$=" FCA"][role="switch"]') != null,
  edit: host.querySelector('button[title="Edit"]') != null,
  remove: host.querySelector('[aria-label="Delete FCA"]') != null,
});

describe("FcaRow write controls", () => {
  it("offers a controller none on an event FCA, but still lists it", async () => {
    const host = await mount(CONTROLLER, fca(7360));
    expect(host.textContent).toContain("EVENT");
    expect(controls(host)).toEqual({ toggle: false, edit: false, remove: false });
  });

  it("offers someone who can only read the plan none on it either", async () => {
    const host = await mount(PLAN_READER, fca(7360));
    expect(controls(host)).toEqual({ toggle: false, edit: false, remove: false });
  });

  it("offers a planner all three on the same event FCA", async () => {
    const host = await mount(PLANNER, fca(7360));
    expect(controls(host)).toEqual({ toggle: true, edit: true, remove: true });
  });

  it("offers the controller all three on an ordinary FCA", async () => {
    const host = await mount(CONTROLLER, fca(null));
    expect(controls(host)).toEqual({ toggle: true, edit: true, remove: true });
  });
});
