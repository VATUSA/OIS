// Premade board layouts. Each template's `build` returns a DashboardState (widgets + grid layout)
// from the airport(s) the user supplies — reusing the same widget kinds the editor produces, so a
// templated board is a fully editable normal board afterwards.

import {AIRPORT_KEY} from "./sources";
import type {DashboardState, FacilityRef, GridCell, Widget} from "./types";

/** What a template's `build` receives: the chosen airports and/or a facility scope. */
export interface TemplateContext {
  icaos: string[];
  facility?: FacilityRef;
}

/** Distributive Omit so a widget spec can drop only `id` while keeping its kind-specific fields. */
type NoId<T> = T extends unknown ? Omit<T, "id"> : never;

interface Placed {
  widget: NoId<Widget>;
  x: number;
  y: number;
  w: number;
  h: number;
}

function board(items: Placed[]): DashboardState {
  const widgets: Widget[] = [];
  const layout: GridCell[] = [];
  for (const it of items) {
    const id = crypto.randomUUID();
    widgets.push({ ...it.widget, id } as Widget);
    layout.push({ i: id, x: it.x, y: it.y, w: it.w, h: it.h });
  }
  return { version: 1, widgets, layout };
}

export interface Template {
  id: string;
  name: string;
  description: string;
  /**
   * What the template needs before it can build.
   *
   * `"national"` needs nothing, like `"none"` — the distinction is who it is *offered* to, because a
   * national board is only useful to someone who works the NAS (see `pages/dashboards/library`).
   */
  airports: "none" | "one" | "many" | "facility" | "national";
  build: (ctx: TemplateContext) => DashboardState;
}

export const TEMPLATES: Template[] = [
  {
    id: "airport-overview",
    name: "Airport overview",
    description: "Arrivals, departures, taxi and the live map for one airport.",
    airports: "one",
    build: ({ icaos: [icao] }) =>
      board([
        { widget: { kind: "stat", metric: "pilots" }, x: 0, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "programs" }, x: 3, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "gdps" }, x: 6, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "fcas" }, x: 9, y: 0, w: 3, h: 2 },
        { widget: { kind: "view", view: "airport-summary", icao }, x: 0, y: 2, w: 6, h: 5 },
        { widget: { kind: "view", view: "departures", icao }, x: 6, y: 2, w: 6, h: 5 },
        { widget: { kind: "map" }, x: 0, y: 7, w: 6, h: 5 },
        { widget: { kind: "view", view: "taxi", icao }, x: 6, y: 7, w: 6, h: 5 },
      ]),
  },
  {
    id: "airport-comparison",
    name: "Airport comparison",
    description: "Compare several airports side by side — inbound counts and arrival tables.",
    airports: "many",
    build: ({ icaos }) => {
      const items: Placed[] = [
        {
          widget: {
            kind: "chart",
            source: "departures",
            params: { icaos },
            chartType: "line",
            x: "status",
            y: [],
            aggregate: "count",
            topN: 0,
          },
          x: 0,
          y: 0,
          w: 12,
          h: 5,
        },
      ];
      icaos.forEach((icao, i) => {
        items.push({
          widget: {
            kind: "table",
            source: "airport-flow",
            params: { icao },
            columns: ["callsign", "status", "eta", "delay_min", "gate"],
          },
          x: (i % 2) * 6,
          y: 5 + Math.floor(i / 2) * 5,
          w: 6,
          h: 5,
        });
      });
      return board(items);
    },
  },
  {
    id: "tmu-watch",
    name: "TMU watch",
    description: "Active programs, TMIs, FCAs and the map — a traffic-management overview.",
    airports: "none",
    build: () =>
      board([
        { widget: { kind: "stat", metric: "gdps" }, x: 0, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "tmis" }, x: 3, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "ground-stops" }, x: 6, y: 0, w: 3, h: 2 },
        { widget: { kind: "stat", metric: "programs" }, x: 9, y: 0, w: 3, h: 2 },
        { widget: { kind: "table", source: "tmis" }, x: 0, y: 2, w: 6, h: 5 },
        { widget: { kind: "table", source: "programs" }, x: 6, y: 2, w: 6, h: 5 },
        { widget: { kind: "table", source: "fcas" }, x: 0, y: 7, w: 6, h: 5 },
        { widget: { kind: "map" }, x: 6, y: 7, w: 6, h: 5 },
      ]),
  },
  {
    id: "traffic-board",
    name: "Traffic board",
    description: "Network traffic at a glance — busiest departures and the live map.",
    airports: "none",
    build: () =>
      board([
        { widget: { kind: "stat", metric: "pilots" }, x: 0, y: 0, w: 4, h: 2 },
        { widget: { kind: "stat", metric: "fcas" }, x: 4, y: 0, w: 4, h: 2 },
        { widget: { kind: "stat", metric: "programs" }, x: 8, y: 0, w: 4, h: 2 },
        {
          widget: {
            kind: "chart",
            source: "traffic",
            chartType: "line",
            x: "dep",
            y: [],
            aggregate: "count",
            topN: 15,
          },
          x: 0,
          y: 2,
          w: 12,
          h: 5,
        },
        { widget: { kind: "map" }, x: 0, y: 7, w: 12, h: 6 },
      ]),
  },
  {
    id: "facility-overview",
    name: "Facility overview",
    description: "Live ATC, aggregated departures & taxi, and a per-airport comparison for one ARTCC or TRACON.",
    airports: "facility",
    build: ({ facility }) => {
      if (!facility) return board([]);
      return board([
        { widget: { kind: "atc", facility }, x: 0, y: 0, w: 4, h: 6 },
        { widget: { kind: "table", source: "departures", params: { facility } }, x: 4, y: 0, w: 8, h: 6 },
        { widget: { kind: "table", source: "taxi", params: { facility } }, x: 0, y: 6, w: 6, h: 5 },
        {
          widget: {
            kind: "chart",
            source: "departures",
            params: { facility },
            chartType: "bar",
            x: AIRPORT_KEY,
            y: [],
            aggregate: "count",
            topN: 0,
          },
          x: 6,
          y: 6,
          w: 6,
          h: 5,
        },
      ]);
    },
  },
  {
    id: "nas-watch",
    name: "NAS watch",
    description:
      "What is overloaded across the country right now — ranked demand, FCA pressure and national ATC.",
    airports: "national",
    // The DCC equivalent of `tmu-watch` (VATUSA/OIS#476), assembling what #474 and #475 added: a
    // national widget scope and a cross-airport overload ranking. Every widget here is one the
    // add-widget menu can produce, so the board stays a normal editable board afterwards.
    build: () =>
      board([
        { widget: { kind: "stat", metric: "pilots" }, x: 0, y: 0, w: 2, h: 2 },
        { widget: { kind: "stat", metric: "programs" }, x: 2, y: 0, w: 2, h: 2 },
        { widget: { kind: "stat", metric: "gdps" }, x: 4, y: 0, w: 2, h: 2 },
        { widget: { kind: "stat", metric: "ground-stops" }, x: 6, y: 0, w: 2, h: 2 },
        { widget: { kind: "stat", metric: "tmis" }, x: 8, y: 0, w: 2, h: 2 },
        { widget: { kind: "stat", metric: "fcas" }, x: 10, y: 0, w: 2, h: 2 },
        // Ranked, not alphabetical: the sort is the feature, and seeding it is what stops the board
        // opening on a table the user has to sort before it answers anything. Same seed the
        // add-widget menu uses, so a template board and a hand-built one agree.
        {
          widget: {
            kind: "table",
            source: "nas-demand",
            sort: [{ id: "exceedance", desc: true }],
          },
          x: 0,
          y: 2,
          w: 7,
          h: 6,
        },
        // The whole country's open positions, from the board `useAtc` already fetches nationally.
        { widget: { kind: "atc", facility: { kind: "national" } }, x: 7, y: 2, w: 5, h: 6 },
        {
          widget: {
            kind: "table",
            source: "nas-fca-pressure",
            sort: [{ id: "count", desc: true }],
          },
          x: 0,
          y: 8,
          w: 7,
          h: 5,
        },
        { widget: { kind: "map" }, x: 7, y: 8, w: 5, h: 5 },
      ]),
  },
];

/**
 * Whether a template asks the user for airports before it can build.
 *
 * An allowlist, not `!== "none"`: a mode that needs no input is not automatically `"none"`.
 * `"national"` needs none either, and the inequality silently prompted the NAS for an ICAO
 * (VATUSA/OIS#476) — so a new no-input mode must opt *in* to prompting rather than inherit it.
 *
 * Lives here beside `templatesFor` for the same reason it does: the rule belongs with the modes it
 * reads, where a test can reach it, not in the page that happens to call it.
 */
export const needsAirports = (t: Template): boolean =>
  t.airports === "one" || t.airports === "many";

/**
 * The templates to offer a user. A national board is only useful to someone who works the NAS, so it
 * is shown only to a `tmu_national` reader — the same flag the national widgets gate on
 * (VATUSA/OIS#474, #475, #476). Curation, not a boundary: every widget on that board can still be
 * added by hand.
 *
 * `undefined` (the profile has not resolved yet) hides it, so it cannot flash to everyone on first
 * paint. Lives here rather than in the page so the rule and the templates it filters cannot drift.
 */
export function templatesFor(tmuNational: boolean | undefined): Template[] {
  return TEMPLATES.filter((t) => t.airports !== "national" || tmuNational === true);
}
