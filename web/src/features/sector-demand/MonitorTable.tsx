import {type ReactNode, useRef, useState} from "react";

import {SectorContextMenu} from "./SectorContextMenu";
import type {SectorDemandRow, SectorDemandTable} from "./sector-demand";
import {
  ALERT_SPAN_BINS,
  BINS_PER_HOUR,
  type ConsolidationPatch,
  type Consolidation,
  DEFAULT_ALERT_BINS,
  DEFAULT_ALERT_ONLY,
  DEFAULT_RANGE_H,
  type DemandTableKind,
  RANGE_MAX_H,
  RANGE_MIN_H,
  type ViewField,
  colourOf,
  consolidateAllPatch,
  deconsolidateAllPatch,
  hoursOf,
  isAlertBins,
  isAlerting,
  isBool,
  isOrder,
  isRange,
  loadStored,
  mapText,
  menuLists,
  moveInOrder,
  orderRows,
  saveStored,
  sectorLabel,
  storageKey,
  zHHMM,
} from "./view";
import {
  C,
  FONT,
  FOOT_H,
  FS,
  FS_LG,
  FS_SM,
  MAP_INPUT_W,
  MAP_W,
  NAME_W,
  ROW_H,
  WEIGHT,
  footerBorder,
  gridBorder,
  inset,
  raised,
  sliderTrack,
} from "./vtbfm-palette";

const FILL = { green: C.green, yellow: C.yellow, red: C.red } as const;

/** A remembered setting of this table, read once at mount and written on every change. */
function useStored<T>(artcc: string, kind: DemandTableKind, field: ViewField, fallback: T, valid: (v: unknown) => v is T) {
  const key = storageKey(artcc, kind, field);
  const [value, setValue] = useState<T>(() => loadStored(key, fallback, valid));
  const set = (next: T) => {
    setValue(next);
    saveStored(key, next);
  };
  return [value, set] as const;
}

/** A neighbour table's open state, which its parent holds because opening one is what fetches it. */
export function useNeighbourOpen(artcc: string, kind: DemandTableKind) {
  return useStored(artcc, kind, "open", false, isBool);
}

/** Writes this table may make. Absent on a neighbour's table and wherever the server says no. */
export type TableEditing = {
  /** Set when the viewer may set this ARTCC's limits (`limits_editable`). */
  onSetMap?: (sector: string, limit: number) => void;
  /** Set when the viewer may change this ARTCC's consolidations (`consolidations_editable`). */
  onConsolidate?: (patch: ConsolidationPatch) => void;
};

export type MonitorTableProps = {
  artcc: string;
  kind: DemandTableKind;
  /** The table's data, or undefined while there is none to draw (`message` says why). */
  table?: SectorDemandTable;
  /** What the table body says instead of a grid when `table` is undefined. */
  message?: string;
  binStartsMs: readonly number[];
  /** Every sector this table can name in its menu, those consolidated away included. */
  universe: readonly string[];
  /** The ARTCC's arrangement as the viewer sees it: the server's plus their unacknowledged writes. */
  consolidation: Consolidation;
  /** MAP values written here and not yet read back, by sector. */
  pendingMaps: Readonly<Record<string, number>>;
  editing?: TableEditing;
  /** A neighbour's table: its open state is the parent's, and it opens and closes whole. */
  neighbour?: { open: boolean; onToggle: () => void };
};

/**
 * One vTBFM monitor table (#794): a 2px outset frame, a title row with the ▼/▶ toggle and the table's
 * name, its controls, then the grid with its bottom time footer. Copied from vTBFM
 * `SectorMonitorPage.tsx:205-503`; every value is in `vtbfm-palette.ts`.
 *
 * On the facility's own table the toggle folds the controls away; on a neighbour's it hides the
 * controls and the grid together, and the table starts closed.
 */
export function MonitorTable({
  artcc,
  kind,
  table,
  message,
  binStartsMs,
  universe,
  consolidation,
  pendingMaps,
  editing,
  neighbour,
}: MonitorTableProps) {
  const title = kind === "tracon" ? `${artcc} TRACON` : artcc;
  const [rangeH, setRangeH] = useStored(artcc, kind, "range", DEFAULT_RANGE_H, isRange);
  const [alertOnly, setAlertOnly] = useStored(artcc, kind, "alertOnly", DEFAULT_ALERT_ONLY, isBool);
  const [alertBins, setAlertBins] = useStored(artcc, kind, "alertSpan", DEFAULT_ALERT_BINS, isAlertBins);
  const [collapsed, setCollapsed] = useStored(artcc, kind, "collapsed", false, isBool);
  const [order, setOrder] = useStored<string[]>(artcc, kind, "order", [], isOrder);
  const [editingSector, setEditingSector] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; sector: string } | null>(null);

  const open = neighbour ? neighbour.open : true;
  const showControls = neighbour ? neighbour.open : !collapsed;
  const sliceN = rangeH * BINS_PER_HOUR;

  // A sector consolidated by this browser's write, not yet read back, already leaves the board.
  const rows = orderRows(
    (table?.rows ?? []).filter((r) => consolidation[r.sector_id] === undefined),
    order,
  );
  const visibleRows = alertOnly ? rows.filter((r) => isAlerting(r, alertBins)) : rows;
  const bins = binStartsMs.slice(0, sliceN);
  const targets = new Set(Object.values(consolidation));
  const nameOf = (r: SectorDemandRow) => sectorLabel(artcc, r.sector_id) + (targets.has(r.sector_id) ? "+" : "");
  const shownMap = (r: SectorDemandRow) => pendingMaps[r.sector_id] ?? r.limit;
  const onSetMap = editing?.onSetMap;
  const onConsolidate = editing?.onConsolidate;

  // A row that left the view (filtered, or consolidated away) takes its editor and its menu with it.
  if (editingSector !== null && !visibleRows.some((r) => r.sector_id === editingSector)) setEditingSector(null);
  if (menu !== null && !rows.some((r) => r.sector_id === menu.sector)) setMenu(null);

  const moveRow = (sector: string, dir: -1 | 1) => {
    const next = moveInOrder(
      order,
      rows.map((r) => r.sector_id),
      visibleRows.map((r) => r.sector_id),
      sector,
      dir,
    );
    if (next) setOrder(next);
  };

  let body: ReactNode;
  if (!table) {
    body = <Message>{message ?? "Waiting for the first sector-monitor cycle…"}</Message>;
  } else if (!table.has_sector_data) {
    body = <Message>{kind === "tracon" ? `No TRACON sector data for ${artcc}` : `No sector data for ${artcc}`}</Message>;
  } else if (visibleRows.length === 0) {
    body = (
      <Message>
        {alertOnly && rows.length > 0
          ? `No ${title} sectors alerting in the next ${hoursOf(alertBins)} h.`
          : `No ${kind === "tracon" ? "TRACON " : ""}sectors for ${artcc}.`}
      </Message>
    );
  } else {
    body = (
      <table
        aria-label={`${title} sector demand`}
        style={{ tableLayout: "fixed", width: "100%", borderCollapse: "collapse", fontFamily: FONT, fontVariantNumeric: "tabular-nums" }}
      >
        <colgroup>
          <col style={{ width: NAME_W }} />
          <col style={{ width: MAP_W }} />
          {bins.map((b) => (
            <col key={b} />
          ))}
        </colgroup>
        <tbody>
          {visibleRows.map((row) => {
            const map = shownMap(row);
            const label = sectorLabel(artcc, row.sector_id);
            return (
              <tr
                key={row.sector_id}
                data-sector={row.sector_id}
                onContextMenu={(e) => {
                  e.preventDefault();
                  // The menu is the consolidation editor; its row moves ride along, as in vTBFM, where
                  // the whole menu is TMU work. Without the right there is no menu at all, rather
                  // than one whose commands can only fail.
                  if (!onConsolidate) return;
                  setMenu({ x: e.clientX, y: e.clientY, sector: row.sector_id });
                }}
              >
                <th
                  scope="row"
                  title={onConsolidate ? "Right-click for row and consolidation commands" : undefined}
                  style={{ ...cell, height: ROW_H, background: C.sectorCyan, padding: "0 2px", whiteSpace: "nowrap", overflow: "hidden", border: gridBorder }}
                >
                  {nameOf(row)}
                </th>
                <th
                  data-map={row.sector_id}
                  onClick={onSetMap ? () => setEditingSector(row.sector_id) : undefined}
                  title={onSetMap ? "Click to edit MAP" : undefined}
                  style={{
                    ...cell,
                    height: ROW_H,
                    background: C.sectorCyan,
                    padding: "0 2px",
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    border: gridBorder,
                    cursor: onSetMap ? "pointer" : "default",
                  }}
                >
                  {onSetMap && editingSector === row.sector_id ? (
                    <MapInput
                      label={`MAP for ${label}`}
                      value={map}
                      onCommit={(n) => {
                        onSetMap(row.sector_id, n);
                        setEditingSector(null);
                      }}
                      onCancel={() => setEditingSector(null)}
                    />
                  ) : (
                    mapText(map)
                  )}
                </th>
                {row.bins.slice(0, sliceN).map((b, i) => {
                  const colour = colourOf(b.level);
                  return (
                    <td
                      key={bins[i] ?? i}
                      data-colour={colour}
                      title={`${label} ${zHHMM(bins[i] ?? 0)}Z · peak ${b.combined} (airborne ${b.active}) vs MAP ${map} · ${colour}`}
                      style={{ ...cell, height: ROW_H, background: FILL[colour], border: gridBorder, padding: 0 }}
                    >
                      {b.combined}
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
        <tfoot>
          <tr>
            <th aria-hidden style={{ background: C.footerCyan, border: "none" }} />
            <th style={{ ...cell, height: FOOT_H, background: C.footerCyan, border: footerBorder, padding: 0 }}>MAP</th>
            {bins.map((b) => (
              <th key={b} style={{ ...cell, height: FOOT_H, background: C.footerCyan, border: footerBorder, padding: 0 }}>
                {zHHMM(b)}
              </th>
            ))}
          </tr>
        </tfoot>
      </table>
    );
  }

  const menuData = (() => {
    if (!menu) return null;
    const lists = menuLists([...rows.map((r) => r.sector_id), ...universe], consolidation, menu.sector);
    const vi = visibleRows.findIndex((r) => r.sector_id === menu.sector);
    return { ...lists, canMoveUp: vi > 0, canMoveDown: vi >= 0 && vi < visibleRows.length - 1 };
  })();

  return (
    <section
      aria-label={`${title} sectors`}
      style={{ ...raised(2), background: C.beige, width: "100%", boxSizing: "border-box", color: C.text, display: "flex", flexDirection: "column" }}
    >
      <div style={{ background: C.beige, ...inset(1) }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "2px 6px" }}>
          <button
            type="button"
            onClick={neighbour ? neighbour.onToggle : () => setCollapsed(!collapsed)}
            className="vtbfm-focus"
            aria-label={neighbour ? `${open ? "Collapse" : "Expand"} ${title}` : `${collapsed ? "Expand" : "Collapse"} ${title} controls`}
            aria-expanded={neighbour ? open : !collapsed}
            style={{ background: "transparent", border: "none", padding: 0, cursor: "pointer", fontSize: FS, color: C.text, lineHeight: "12px" }}
          >
            {showControls ? "▼" : "▶"}
          </button>
          <span style={{ fontFamily: FONT, fontSize: FS, fontWeight: WEIGHT }}>{title}</span>
        </div>
        {showControls && (
          <div style={{ padding: "4px 8px 6px", display: "flex", flexDirection: "column", gap: 3, alignItems: "flex-start", fontFamily: FONT }}>
            <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
              <div>
                <div style={{ fontSize: FS, fontWeight: WEIGHT, lineHeight: "13px" }}>Time Range:</div>
                <div style={{ fontSize: FS, fontWeight: WEIGHT, lineHeight: "13px" }}>{rangeH.toFixed(2)} hours.</div>
              </div>
              <TimeRange label={`${title} time range (hours)`} hours={rangeH} onChange={setRangeH} />
            </div>
            <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: FS_LG, fontWeight: WEIGHT }}>
              <input
                type="checkbox"
                aria-label={`${title}: show if alerted`}
                checked={alertOnly}
                onChange={(e) => setAlertOnly(e.target.checked)}
                className="vtbfm-focus"
              />
              <span>Show if alerted in next:</span>
              <select
                aria-label={`${title} alert span (hours)`}
                value={alertBins}
                onChange={(e) => setAlertBins(Number(e.target.value))}
                className="vtbfm-focus"
                style={{ fontFamily: FONT, fontSize: FS, ...inset(1), background: C.field, color: C.text }}
              >
                {ALERT_SPAN_BINS.map((n) => (
                  <option key={n} value={n}>
                    {hoursOf(n)}
                  </option>
                ))}
              </select>
              <span>hours (Flow Limit)</span>
            </label>
          </div>
        )}
      </div>
      {open && <div style={{ overflowX: "auto", border: `1px solid ${C.outer}`, background: C.beige }}>{body}</div>}
      {menu && menuData && onConsolidate && (
        <SectorContextMenu
          x={menu.x}
          y={menu.y}
          target={menu.sector}
          center={artcc}
          sectors={menuData.offered}
          consolidatedHere={menuData.consolidatedHere}
          hasAnyConsolidation={Object.keys(consolidation).length > 0}
          canMoveUp={menuData.canMoveUp}
          canMoveDown={menuData.canMoveDown}
          onMoveUp={() => moveRow(menu.sector, -1)}
          onMoveDown={() => moveRow(menu.sector, 1)}
          onConsolidateAll={() => onConsolidate(consolidateAllPatch(menuData.items, consolidation, menu.sector, false))}
          onConsolidateAllExceptConsolidated={() => onConsolidate(consolidateAllPatch(menuData.items, consolidation, menu.sector, true))}
          onDeconsolidateAllFromTarget={() => onConsolidate(deconsolidateAllPatch(consolidation, menu.sector, "target"))}
          onDeconsolidateAllInCenter={() => onConsolidate(deconsolidateAllPatch(consolidation, menu.sector, "center"))}
          onReleaseSector={(s) => onConsolidate({ [s]: null })}
          onToggleSector={(s) => onConsolidate({ [s]: consolidation[s] === menu.sector ? null : menu.sector })}
          onClose={() => setMenu(null)}
        />
      )}
    </section>
  );
}

const cell = { color: C.text, textAlign: "center", verticalAlign: "middle", fontFamily: FONT, fontSize: FS, fontWeight: WEIGHT } as const;

function Message({ children }: { children: ReactNode }) {
  return <div style={{ padding: 20, textAlign: "center", fontFamily: FONT, fontSize: FS }}>{children}</div>;
}

function TimeRange({ label, hours, onChange }: { label: string; hours: number; onChange: (h: number) => void }) {
  const pct = ((hours - RANGE_MIN_H) / (RANGE_MAX_H - RANGE_MIN_H)) * 100;
  const stops = Array.from({ length: RANGE_MAX_H - RANGE_MIN_H + 1 }, (_, i) => RANGE_MIN_H + i);
  return (
    <div style={{ fontFamily: FONT, width: 130 }}>
      <input
        type="range"
        min={RANGE_MIN_H}
        max={RANGE_MAX_H}
        step={1}
        value={hours}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label={label}
        className="vtbfm-range vtbfm-focus"
        style={{ width: 130, display: "block", background: sliderTrack(pct) }}
      />
      <div style={{ width: 130, display: "flex", justifyContent: "space-between", fontSize: FS_SM, color: C.text, marginTop: 1 }}>
        {stops.map((n) => (
          <span key={n} style={{ width: 7, textAlign: "center" }}>
            {n}
          </span>
        ))}
      </div>
    </div>
  );
}

/** The inline MAP editor: Enter or blur commits only a changed positive number; Escape cancels. */
function MapInput({ label, value, onCommit, onCancel }: { label: string; value: number; onCommit: (n: number) => void; onCancel: () => void }) {
  const [draft, setDraft] = useState(String(value));
  const done = useRef(false);
  const finish = () => {
    if (done.current) return;
    done.current = true;
    const n = Math.round(Number(draft.trim()));
    if (draft.trim() !== "" && Number.isFinite(n) && n > 0 && n !== value) onCommit(n);
    else onCancel();
  };
  return (
    <input
      type="number"
      min={1}
      aria-label={label}
      value={draft}
      autoFocus
      onChange={(e) => setDraft(e.target.value)}
      onBlur={finish}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          finish();
        } else if (e.key === "Escape") {
          e.preventDefault();
          done.current = true;
          onCancel();
        }
      }}
      className="vtbfm-focus"
      style={{ width: MAP_INPUT_W, ...inset(1), background: C.field, fontFamily: FONT, fontSize: FS, textAlign: "center", color: C.text }}
    />
  );
}
