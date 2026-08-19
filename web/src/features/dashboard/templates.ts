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
  /** What the template needs before it can build. */
  airports: "none" | "one" | "many" | "facility";
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
];
