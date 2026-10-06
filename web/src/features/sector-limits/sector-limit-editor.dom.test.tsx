// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` in a test is
// always too late (VATUSA/OIS#387) — mock the client itself.
vi.mock("@/lib/api", () => ({ois: {GET: get, PUT: put}, API_BASE: ""}));

import {SectorLimitEditor} from "./SectorLimitEditor";
import {type SectorLimits, sectorLimitsKey} from "./sector-limits";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
  get.mockReset();
  put.mockReset();
});

/** ZDC01 carries an override of 14; ZDC02 reads the default. */
const limits = (artcc: string, editable: boolean): SectorLimits => ({
  artcc,
  default_limit: 10,
  editable,
  sectors: [
    {sector_id: `${artcc}01`, tier: "high", limit: 14, overridden: true},
    {sector_id: `${artcc}02`, tier: "low", limit: 10, overridden: false},
  ],
});

/** Mounts the editor against a seeded cache — nothing reaches the network. */
async function mount(artcc: string, seeded: SectorLimits) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {retry: false, refetchOnMount: false, refetchOnWindowFocus: false, staleTime: Infinity},
      mutations: {retry: false},
    },
  });
  qc.setQueryData(sectorLimitsKey(seeded.artcc), seeded);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <SectorLimitEditor artcc={artcc} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

const limitInput = (host: HTMLElement, sectorId: string) =>
  host.querySelector<HTMLInputElement>(`input[aria-label="Limit for ${sectorId}"]`);

/** Sets an input through React's own value tracking. */
const type = (input: HTMLInputElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });
const key = (input: HTMLInputElement, k: string) =>
  act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", {key: k, bubbles: true}));
  });
const focus = (input: HTMLInputElement) => act(async () => input.focus());
const blur = (input: HTMLInputElement) => act(async () => input.blur());

/** Lets the mutation's promise chain (PUT → cache write → invalidation) settle. */
const settle = () => act(async () => new Promise((r) => setTimeout(r, 0)));

describe("SectorLimitEditor (#722)", () => {
  it("lists the ARTCC's sectors with tier, limit and whether each is overridden", async () => {
    const {host} = await mount("ZDC", limits("ZDC", true));
    const rows = [...host.querySelectorAll("tbody tr")].map((tr) => tr.textContent);
    expect(rows).toEqual(["ZDC01HighOverride", "ZDC02LowDefault"]);
    expect(limitInput(host, "ZDC01")!.value).toBe("14");
    expect(limitInput(host, "ZDC02")!.value).toBe("10");
  });

  it("renders a neighbour's limits as inert text: no input, nothing focusable, no hover or cursor hint", async () => {
    const {host} = await mount("ZNY", limits("ZNY", false));
    const body = host.querySelector("tbody")!;
    expect(body.textContent).toContain("14");
    expect(body.querySelectorAll("input, button, select, textarea, [tabindex], [contenteditable], [role=button]")).toHaveLength(0);
    for (const el of [body, ...body.querySelectorAll<HTMLElement>("*")]) {
      expect(el.getAttribute("class") ?? "", el.outerHTML).not.toMatch(/\b(hover|focus|cursor)[:-]/);
      expect(el.getAttribute("title"), el.outerHTML).toBeNull();
    }
    // The same sectors for the viewer's own facility are inputs — the difference is the editable flag.
    const own = await mount("ZDC", limits("ZDC", true));
    expect(limitInput(own.host, "ZDC01")).not.toBeNull();
  });

  // Each entry either fails "positive whole number" or equals the stored value; none may reach the API
  // (#722: an invalid entry cancels without writing and without clearing an existing override).
  it.each([
    ["zero", "0"],
    ["negative", "-3"],
    ["non-numeric", "abc"],
    ["empty", ""],
    ["fractional", "1.5"],
    ["unchanged", "14"],
  ])("cancels a %s entry on Enter and on blur: no PUT, the override stands", async (_case, entry) => {
    const {host, qc} = await mount("ZDC", limits("ZDC", true));
    for (const commit of [(i: HTMLInputElement) => key(i, "Enter"), blur]) {
      const input = limitInput(host, "ZDC01")!;
      await focus(input);
      await type(input, entry);
      await commit(input);
      await settle();
      expect(put).not.toHaveBeenCalled();
      expect(limitInput(host, "ZDC01")!.value).toBe("14");
    }
    expect(qc.getQueryData<SectorLimits>(sectorLimitsKey("ZDC"))!.sectors[0]).toEqual({
      sector_id: "ZDC01",
      tier: "high",
      limit: 14,
      overridden: true,
    });
    expect(host.querySelector("tbody tr")!.textContent).toContain("Override");
  });

  it("Escape discards a valid draft without a write", async () => {
    const {host} = await mount("ZDC", limits("ZDC", true));
    const input = limitInput(host, "ZDC01")!;
    await focus(input);
    await type(input, "20");
    await key(input, "Escape");
    await settle();
    expect(put).not.toHaveBeenCalled();
    expect(input.value).toBe("14");
  });

  it("a valid change sends exactly one PUT with { limit } and shows the saved value", async () => {
    const saved = {sector_id: "ZDC02", tier: "low", limit: 12, overridden: true};
    put.mockResolvedValue({data: saved});
    const after = limits("ZDC", true);
    after.sectors[1] = saved;
    get.mockResolvedValue({data: after});
    const {host, qc} = await mount("ZDC", limits("ZDC", true));
    const input = limitInput(host, "ZDC02")!;
    await focus(input);
    await type(input, "12");
    await key(input, "Enter"); // Enter blurs; the blur is the one commit.
    await settle();
    expect(put).toHaveBeenCalledTimes(1);
    expect(put).toHaveBeenCalledWith("/api/v1/flow/sector-limits/{artcc}/{sector_id}", {
      params: {path: {artcc: "ZDC", sector_id: "ZDC02"}},
      body: {limit: 12},
    });
    expect(qc.getQueryData<SectorLimits>(sectorLimitsKey("ZDC"))!.sectors[1]).toEqual(saved);
    expect(limitInput(host, "ZDC02")!.value).toBe("12");
    expect(host.querySelectorAll("tbody tr")[1].textContent).toContain("Override");
  });

  it("setting the default over an override is a write (the server's reset), not a cancel", async () => {
    put.mockResolvedValue({data: {sector_id: "ZDC01", tier: "high", limit: 10, overridden: false}});
    get.mockResolvedValue({data: limits("ZDC", true)});
    const {host} = await mount("ZDC", limits("ZDC", true));
    const input = limitInput(host, "ZDC01")!;
    await focus(input);
    await type(input, "10");
    await blur(input);
    await settle();
    expect(put).toHaveBeenCalledTimes(1);
    expect(put.mock.calls[0][1]).toMatchObject({body: {limit: 10}});
  });

  it("a refused write puts the stored value back and says so", async () => {
    put.mockResolvedValue({error: {error: "forbidden"}});
    const {host} = await mount("ZDC", limits("ZDC", true));
    const input = limitInput(host, "ZDC01")!;
    await focus(input);
    await type(input, "20");
    await key(input, "Enter");
    await settle();
    expect(put).toHaveBeenCalledTimes(1);
    expect(limitInput(host, "ZDC01")!.value).toBe("14");
    expect(document.body.textContent).toContain("Couldn't set ZDC01's limit");
  });

  it("a toast does not remount the other inputs: a draft in another sector survives a refused write", async () => {
    put.mockResolvedValue({error: {error: "forbidden"}});
    const {host} = await mount("ZDC", limits("ZDC", true));
    // A draft held in ZDC02 (typed, not yet committed) …
    await type(limitInput(host, "ZDC02")!, "17");
    // … while a write on ZDC01 is refused and raises a toast.
    const first = limitInput(host, "ZDC01")!;
    await focus(first);
    await type(first, "20");
    await key(first, "Enter");
    await settle();
    expect(document.body.textContent).toContain("Couldn't set ZDC01's limit");
    expect(limitInput(host, "ZDC01")!.value).toBe("14");
    expect(limitInput(host, "ZDC02")!.value).toBe("17");
  });

  it("names the ARTCC when it has no sector data, instead of drawing an empty table", async () => {
    const {host} = await mount("zla ", {artcc: "ZLA", default_limit: 10, editable: true, sectors: []});
    expect(host.textContent).toContain("No sector data for ZLA");
    expect(host.querySelector("table")).toBeNull();
  });
});
