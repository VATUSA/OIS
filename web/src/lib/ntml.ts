import type {components} from "@ois/api-client";

export type Ntml = components["schemas"]["NtmlRestriction"];
export type Bound = components["schemas"]["Bound"];

export const DIRECTIONS = ["arrivals", "departures", "enroute"] as const;
export const KINDS = ["MIT", "MINIT", "STOP", "DSP", "APREQ", "TBM", "CFR", "TXT"] as const;
export const AIRCRAFT = ["", "ALL", "JET", "PROP", "TURBOPROP"] as const;
export const QUALIFIERS = [
  "",
  "AS ONE",
  "EACH",
  "EVERY OTHER",
  "NO STACKS",
  "SINGLE STREAM",
  "PER AIRPORT",
  "PER FIX",
  "PER ROUTE",
  "PER STRAT",
  "PER STREAM",
] as const;
export const SPEED_OPS = ["=", "≤", "≥"] as const;
export const ALT_OPS = ["AT", "AOB", "AOA"] as const;

export const EMPTY_NTML: Ntml = {
  element: "",
  direction: "arrivals",
  kind: "MIT",
  value: 20,
  via: null,
  text: null,
  qualifier: null,
  aircraft: null,
  speed: null,
  altitude: null,
  condition: null,
  condition_detail: null,
  exclude: [],
};

const clean = (s: string) => s.trim().toUpperCase();
const opt = (s?: string | null): string | null => {
  const t = (s ?? "").trim();
  return t ? t : null;
};

/** Mirror of `backend/src/tmi.rs` `encode` — the canonical raw NTML line, for a live preview. */
export function encodeNtml(r: Ntml): string {
  const parts: string[] = [clean(r.element)];
  if (r.direction === "arrivals") parts.push("arrivals");
  else if (r.direction === "departures") parts.push("departures");
  const via = opt(r.via);
  if (via) parts.push(`via ${clean(via)}`);

  const kind = clean(r.kind);
  if (kind === "MIT" || kind === "MINIT") parts.push(`${r.value ?? 0}${kind}`);
  else if (kind === "TXT") {
    const t = opt(r.text);
    if (t) parts.push(t);
  } else parts.push(kind);

  const q = opt(r.qualifier);
  if (q) parts.push(clean(q));
  const a = opt(r.aircraft);
  if (a) parts.push(`TYPE:${clean(a)}`);
  if (r.speed) parts.push(`SPD:${r.speed.op}${r.speed.value}`);
  if (r.altitude) parts.push(`ALT:${clean(r.altitude.op)}${String(r.altitude.value).padStart(3, "0")}`);
  const c = opt(r.condition);
  if (c) {
    const d = opt(r.condition_detail);
    parts.push(d ? `${clean(c)}:${clean(d)}` : clean(c));
  }
  const excl = (r.exclude ?? []).map(clean).filter(Boolean);
  if (excl.length) parts.push(`EXCL:${excl.join(",")}`);
  return parts.join(" ");
}

const AIRCRAFT_EN: Record<string, string> = {
  ALL: "all aircraft",
  JET: "jets only",
  PROP: "props only",
  TURBOPROP: "turboprops only",
};

/** Mirror of `backend/src/tmi.rs` `render_english` — the decoded, pilot-facing sentence. */
export function decodeNtml(r: Ntml): string {
  let s = clean(r.element);
  if (r.direction === "arrivals") s += " arrivals";
  else if (r.direction === "departures") s += " departures";
  const via = opt(r.via);
  if (via) s += ` via ${clean(via)}`;
  s += ": ";

  const kind = clean(r.kind);
  s +=
    kind === "MIT"
      ? `${r.value ?? 0} miles-in-trail`
      : kind === "MINIT"
        ? `${r.value ?? 0} minutes-in-trail`
        : kind === "STOP"
          ? "stop"
          : kind === "APREQ"
            ? "approval request (APREQ)"
            : kind === "CFR"
              ? "call-for-release (CFR)"
              : kind === "DSP"
                ? "departure spacing program (DSP)"
                : kind === "TBM"
                  ? "time-based metering (TBM)"
                  : kind === "TXT"
                    ? (opt(r.text) ?? "free-text restriction")
                    : kind.toLowerCase();

  const mods: string[] = [];
  const q = opt(r.qualifier);
  if (q) mods.push(q.toLowerCase());
  const a = opt(r.aircraft);
  if (a) mods.push(AIRCRAFT_EN[clean(a)] ?? a.toLowerCase());
  if (r.speed) {
    const lead = r.speed.op === "≤" ? "at or below " : r.speed.op === "≥" ? "at or above " : "at ";
    mods.push(`${lead}${r.speed.value}kt`);
  }
  if (r.altitude) {
    const op = clean(r.altitude.op);
    const lead = op === "AOB" ? "at or below " : op === "AOA" ? "at or above " : "at ";
    mods.push(`${lead}FL${String(r.altitude.value).padStart(3, "0")}`);
  }
  if (mods.length) s += ` (${mods.join(", ")})`;

  const c = opt(r.condition);
  if (c) {
    const d = opt(r.condition_detail);
    const cond = d && d.toUpperCase() !== c.toUpperCase() ? `${c.toLowerCase()} (${d.toLowerCase()})` : c.toLowerCase();
    s += ` — due to ${cond}`;
  }
  const excl = (r.exclude ?? []).map(clean).filter(Boolean);
  if (excl.length) s += `; excluding ${excl.join(", ")}`;
  return s;
}
