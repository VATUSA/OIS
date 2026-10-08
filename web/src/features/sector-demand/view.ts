import type {SectorDemandBin, SectorDemandRow} from "./sector-demand";

/** Which of an ARTCC's two tables (#725): Low/High/Ultra High, or Approach Control. */
export type DemandTableKind = "enroute" | "tracon";

/** Every bin is a quarter-hour. */
export const BINS_PER_HOUR = 4;
/** The Time Range slider's stops, in whole hours (vTBFM `SectorMonitorPage.tsx:63-64`). */
export const RANGE_MIN_H = 2;
export const RANGE_MAX_H = 6;
export const DEFAULT_RANGE_H = 4;
/** "Show if alerted in next:" choices, in bins: 1.00, 1.50, 2.00, 2.25, 3.00, 4.00, 5.00, 6.00 h. */
export const ALERT_SPAN_BINS: readonly number[] = [4, 6, 8, 9, 12, 16, 20, 24];
/** The filter starts on at 2 h (owner decision on #725, Q4; vTBFM starts it off). */
export const DEFAULT_ALERT_ONLY = true;
export const DEFAULT_ALERT_BINS = 8;

/** `2.00` — vTBFM's hours figure, from a bin count. */
export const hoursOf = (bins: number) => (bins / BINS_PER_HOUR).toFixed(2);

const pad = (n: number) => String(n).padStart(2, "0");
/** `0415` — a bin's start in Zulu, no Z. */
export const zHHMM = (ms: number) => {
  const d = new Date(ms);
  return `${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}`;
};
/** The MAP cell: `10/10`, `02/02`. */
export const mapText = (limit: number) => `${pad(limit)}/${pad(limit)}`;

/** The colour word a bin's level reads as: the server judged it (strict `>`, #722); this only names it. */
export type BinColour = "green" | "yellow" | "red";
export const colourOf = (level: SectorDemandBin["level"]): BinColour =>
  level === "over" ? "red" : level === "watch" ? "yellow" : "green";

/** The board's name for a sector: `ZLA25`. */
export const sectorLabel = (artcc: string, sector: string) => `${artcc}${sector}`;

/** vTBFM's sector order: numeric when both are numbers (16 before 100), else lexical. */
export function cmpSector(x: string, y: string): number {
  const nx = Number(x);
  const ny = Number(y);
  if (x.trim() !== "" && y.trim() !== "" && !Number.isNaN(nx) && !Number.isNaN(ny)) return nx - ny;
  return x < y ? -1 : x > y ? 1 : 0;
}

/** `rows` sorted by this browser's `order` first, then canonically; a sector not in `order` goes last. */
export function orderRows<T extends {sector_id: string}>(rows: readonly T[], order: readonly string[]): T[] {
  const at = new Map(order.map((s, i) => [s, i]));
  const ix = (s: string) => at.get(s) ?? Number.MAX_SAFE_INTEGER;
  return rows.slice().sort((a, b) => ix(a.sector_id) - ix(b.sector_id) || cmpSector(a.sector_id, b.sector_id));
}

/** Whether a row has a non-green bin among its first `alertBins`. Independent of the drawn range. */
export const isAlerting = (row: SectorDemandRow, alertBins: number) =>
  row.bins.slice(0, alertBins).some((b) => b.level !== "ok");

/** `order` with `sector` moved one place past its visible neighbour, keeping every other sector put. */
export function moveInOrder(
  order: readonly string[],
  all: readonly string[],
  visible: readonly string[],
  sector: string,
  dir: -1 | 1,
): string[] | null {
  const i = visible.indexOf(sector);
  const anchor = visible[i + dir];
  if (i < 0 || anchor === undefined) return null;
  const out = [...new Set([...order, ...all])].filter((s) => s !== sector);
  const at = out.indexOf(anchor);
  if (at < 0) return null;
  out.splice(dir === 1 ? at + 1 : at, 0, sector);
  return out;
}

// --- Consolidation ----------------------------------------------------------------------------------

/** Source sector → the sector it is worked at, for one ARTCC. */
export type Consolidation = Readonly<Record<string, string>>;
/** A batch for `PUT /flow/sector-consolidations/{artcc}`: a sector → its target, or null to release. */
export type ConsolidationPatch = Record<string, string | null>;

/** The server's arrangement, read off the rows: each row lists the sectors worked at it. */
export function consolidationOf(rows: readonly SectorDemandRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const r of rows) for (const src of r.consolidated) out[src] = r.sector_id;
  return out;
}

/** `base` with this browser's unacknowledged writes laid over it. */
export function withPending(base: Consolidation, pending: Readonly<ConsolidationPatch>): Record<string, string> {
  const out: Record<string, string> = {...base};
  for (const [k, v] of Object.entries(pending)) {
    if (v === null) delete out[k];
    else out[k] = v;
  }
  return out;
}

/** One entry of the menu's sector lists. */
export type MenuSector = {sector: string; checked: boolean};

/**
 * What the menu opened on `target` offers (vTBFM `SectorMonitorPage.tsx:282-327`). `universe` is every
 * sector of the table, those consolidated away included.
 */
export function menuLists(universe: readonly string[], cons: Consolidation, target: string) {
  const items: MenuSector[] = [...new Set(universe)]
    .sort(cmpSector)
    .filter((s) => s !== target)
    .map((s) => ({sector: s, checked: cons[s] === target}));
  // Every sector except one already worked at ANOTHER position: a parent is offered, its sources aren't.
  const offered = items.filter((it) => cons[it.sector] === undefined || cons[it.sector] === target);
  const consolidatedHere = items.filter((it) => cons[it.sector] === target).map((it) => ({...it, checked: true}));
  return {items, offered, consolidatedHere};
}

/** Consolidate All into `target`; `exceptConsolidated` skips every sector already in an arrangement. */
export function consolidateAllPatch(
  items: readonly MenuSector[],
  cons: Consolidation,
  target: string,
  exceptConsolidated: boolean,
): ConsolidationPatch {
  const targets = new Set(Object.values(cons));
  const patch: ConsolidationPatch = {};
  for (const {sector} of items) {
    if (sector === target || cons[sector] === target) continue;
    if (exceptConsolidated && (cons[sector] !== undefined || targets.has(sector))) continue;
    patch[sector] = target;
  }
  return patch;
}

/** Deconsolidate All from `target` (`scope: "target"`), or every arrangement in the ARTCC. */
export function deconsolidateAllPatch(cons: Consolidation, target: string, scope: "target" | "center"): ConsolidationPatch {
  const patch: ConsolidationPatch = {};
  for (const [k, v] of Object.entries(cons)) {
    if (scope === "target" && v !== target) continue;
    patch[k] = null;
  }
  return patch;
}

/**
 * The line a refused consolidation write shows (#792): the server says only the status, so the sector
 * is named from the patch that was sent and the arrangement it was sent against.
 */
export function consolidationError(
  status: number | undefined,
  artcc: string,
  patch: Readonly<ConsolidationPatch>,
  cons: Consolidation,
  known: ReadonlySet<string>,
): string {
  const name = (s: string) => sectorLabel(artcc, s);
  const sets = Object.entries(patch).filter((e): e is [string, string] => e[1] !== null);
  if (status === 400) {
    const self = sets.find(([s, t]) => s === t);
    if (self) return `${name(self[0])} can't be consolidated into itself.`;
  }
  if (status === 409) {
    // The server refuses a save whose target is worked at its source. With one save in the patch that
    // is exactly what happened, even when another TMU's change hasn't reached this board yet.
    const loop = sets.find(([s, t]) => cons[t] === s) ?? (sets.length === 1 ? sets[0] : undefined);
    if (loop) return `Can't consolidate ${name(loop[0])} into ${name(loop[1])}: ${name(loop[1])} is worked at ${name(loop[0])}.`;
    return "Those consolidations would make a loop; nothing was saved.";
  }
  if (status === 404) {
    const unknown = sets.flat().find((s) => !known.has(s));
    if (unknown !== undefined) return `${name(unknown)} is not one of ${artcc}'s sectors.`;
    return `A sector in that change is no longer one of ${artcc}'s sectors; nothing was saved.`;
  }
  if (status === 403) return `You can't change ${artcc}'s consolidations.`;
  return "Could not save the consolidation — check TMU access / connection.";
}

// --- Per-browser memory (#794 Q7: per ARTCC and table) ----------------------------------------------
//
// View state, not a synced setting, so it lives in this browser's localStorage under
// `ois.sectorMonitor.{ARTCC}.{enroute|tracon}.{field}`, as vTBFM keeps `vtbfm-mon-{artcc}-*`. Storage
// can be missing or throw (private mode, blocked site data), and the page must work without it, so
// every access is guarded and a bad value reads as the default.

export type ViewField = "open" | "range" | "alertOnly" | "alertSpan" | "collapsed" | "order";

export const storageKey = (artcc: string, kind: DemandTableKind, field: ViewField) =>
  `ois.sectorMonitor.${artcc}.${kind}.${field}`;

/** A remembered value, or `fallback` when it is missing, unreadable or fails `valid`. */
export function loadStored<T>(key: string, fallback: T, valid: (v: unknown) => v is T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const v: unknown = JSON.parse(raw);
    return valid(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

export function saveStored(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* non-fatal: the view just isn't remembered */
  }
}

export const isBool = (v: unknown): v is boolean => typeof v === "boolean";
export const isRange = (v: unknown): v is number =>
  typeof v === "number" && Number.isInteger(v) && v >= RANGE_MIN_H && v <= RANGE_MAX_H;
export const isAlertBins = (v: unknown): v is number => typeof v === "number" && ALERT_SPAN_BINS.includes(v);
export const isOrder = (v: unknown): v is string[] => Array.isArray(v) && v.every((s) => typeof s === "string");
