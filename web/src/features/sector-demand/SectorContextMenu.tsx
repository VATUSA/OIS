import {type CSSProperties, type ReactNode, useEffect, useLayoutEffect, useRef, useState} from "react";
import {createPortal} from "react-dom";

import type {MenuSector} from "./view";
import {FONT, MENU, MENU_BEVEL, MENU_ROW_RAISED} from "./vtbfm-palette";

// The Sector Monitor's row menu (#794), ported from vTBFM's `SectorContextMenu.tsx` at `b7328138`:
// the same three levels, the same raised cream frame, the same 180 ms submenus.
//
// - A submenu is a DOM child of the row that opens it, absolutely positioned at `left: 100%`, so CSS
//   anchors it and the pointer never leaves the row's subtree on the way across.
// - Escape unwinds one level at a time; a mousedown outside the menu closes all of it.
// - The two sector lists are checklists: a click toggles and the menu stays open. Every other
//   command runs and closes, like a desktop menu.
// - Consolidate and Deconsolidate are mirrors: All into T / All from T, All into T Except Consolidated /
//   All in ZLA, into T ▸ / from T ▸.

/** Which branch of the menu has its second level open. */
type Branch = "consolidate" | "deconsolidate";

const surface: CSSProperties = {
  boxSizing: "border-box",
  background: MENU.surface,
  boxShadow: MENU_BEVEL,
  borderRadius: 0,
  paddingTop: MENU.bevelTL,
  paddingLeft: MENU.bevelTL,
  paddingRight: MENU.bevelBR,
  paddingBottom: MENU.bevelBR,
  fontFamily: FONT,
  fontSize: MENU.fontPx,
  lineHeight: 1,
  color: MENU.text,
  cursor: "default",
  userSelect: "none",
  whiteSpace: "nowrap",
  width: "max-content",
};

/** The submenu arrow, three-toned like vTBFM's; flat when the row is disabled. */
function Arrow({ dim }: { dim?: boolean }) {
  return (
    <svg width="10" height="12" viewBox="0 0 9 11" aria-hidden="true" style={{ display: "block" }}>
      {dim ? (
        <polygon points="0,0 9,5.5 0,11" fill={MENU.disabledText} />
      ) : (
        <>
          <polygon points="0,0 9,5.5 0,11" fill={MENU.arrowFace} />
          <polygon points="0,0 8.2,5 0,3.6" fill={MENU.arrowLight} />
          <polygon points="0,7.4 8.2,6 0,11" fill={MENU.arrowDark} />
        </>
      )}
    </svg>
  );
}

function Check() {
  return (
    <svg width="11" height="10" viewBox="0 0 11 10" aria-hidden="true" style={{ display: "block" }}>
      <polyline points="0,4.8 3.6,8.6 11,0.6" fill="none" stroke={MENU.text} strokeWidth="1.8" />
    </svg>
  );
}

/** Level 1: fixed at the click, nudged to stay on screen before paint. */
function RootPanel({ x, y, children }: { x: number; y: number; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [left, setLeft] = useState(x);
  const [top, setTop] = useState(y);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setLeft(Math.max(MENU.edge, Math.min(x, window.innerWidth - el.offsetWidth - MENU.edge)));
    setTop(Math.max(MENU.edge, Math.min(y, window.innerHeight - el.offsetHeight - MENU.edge)));
  }, [x, y]);
  return (
    <div
      ref={ref}
      role="menu"
      style={{ ...surface, position: "fixed", left, top, zIndex: 9999 }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {children}
    </div>
  );
}

/** A child level, anchored to its row by CSS; measured only to flip left or slide up at the edges. */
function Submenu({ scroll, children }: { scroll?: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [flip, setFlip] = useState(false);
  const [dy, setDy] = useState(0);
  useLayoutEffect(() => {
    const place = () => {
      const el = ref.current;
      const row = el?.parentElement;
      if (!el || !row) return;
      const r = row.getBoundingClientRect();
      const w = el.offsetWidth;
      const h = el.offsetHeight;
      setFlip(r.right + w > window.innerWidth - MENU.edge && r.left - w >= MENU.edge);
      const naturalTop = r.top - MENU.bevelTL;
      const maxTop = window.innerHeight - h - MENU.edge;
      setDy(naturalTop > maxTop ? Math.max(MENU.edge, maxTop) - naturalTop : 0);
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, []);
  const style: CSSProperties = {
    ...surface,
    position: "absolute",
    top: -MENU.bevelTL,
    marginTop: dy,
    ...(flip ? { right: "100%" } : { left: "100%" }),
    zIndex: 1,
    // A facility can have forty sectors; the list scrolls rather than run off a laptop screen.
    ...(scroll ? { maxHeight: `calc(100vh - ${MENU.edge * 4}px)`, overflowY: "auto" } : {}),
  };
  return (
    <div
      ref={ref}
      role="menu"
      style={style}
      // The submenu lives inside its parent row; a click here must not also fire the row's handler.
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {children}
    </div>
  );
}

function Row({
  label,
  arrow,
  checked,
  checkColumn,
  hovered,
  disabled,
  onEnter,
  onClick,
  children,
}: {
  label: string;
  arrow?: boolean;
  checked?: boolean;
  /** Reserve the checkmark column even when unchecked, so every name starts at the same x. */
  checkColumn?: boolean;
  hovered: boolean;
  disabled?: boolean;
  onEnter: () => void;
  onClick: () => void;
  children?: ReactNode;
}) {
  const up = hovered && !disabled;
  return (
    <div
      role={checkColumn ? "menuitemcheckbox" : "menuitem"}
      aria-disabled={disabled || undefined}
      aria-checked={checkColumn ? !!checked : undefined}
      aria-haspopup={arrow || undefined}
      onMouseEnter={onEnter}
      onClick={disabled ? undefined : onClick}
      style={{
        display: "flex",
        alignItems: "center",
        height: MENU.rowH,
        boxSizing: "border-box",
        paddingLeft: MENU.textInset - MENU.bevelTL,
        paddingRight: MENU.arrowRight - MENU.bevelBR,
        background: up ? MENU.rowRaisedBg : undefined,
        boxShadow: up ? MENU_ROW_RAISED : undefined,
        color: disabled ? MENU.disabledText : MENU.text,
        position: "relative",
      }}
    >
      {checkColumn && (
        <span style={{ width: MENU.checkCol, flex: `0 0 ${MENU.checkCol}px`, display: "flex", alignItems: "center" }}>
          {checked ? <Check /> : null}
        </span>
      )}
      <span>{label}</span>
      {arrow && <span style={{ width: MENU.arrowCol, flex: `0 0 ${MENU.arrowCol}px` }} />}
      {arrow && (
        <span style={{ position: "absolute", right: MENU.arrowRight - MENU.bevelBR, top: "50%", transform: "translateY(-50%)" }}>
          <Arrow dim={disabled} />
        </span>
      )}
      {children}
    </div>
  );
}

export type SectorContextMenuProps = {
  /** Viewport coordinates of the right-click. */
  x: number;
  y: number;
  /** The sector the menu was opened on, bare (`30`, never `30+`). */
  target: string;
  /** The ARTCC, upper-case, named by the ARTCC-wide release. */
  center: string;
  /** Consolidate into T's checklist: every other sector not already worked at another position. */
  sectors: readonly MenuSector[];
  /** Deconsolidate from T's checklist: the sectors worked at the target. */
  consolidatedHere: readonly MenuSector[];
  /** Whether the ARTCC has any arrangement at all; gates the whole Deconsolidate branch. */
  hasAnyConsolidation: boolean;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onConsolidateAll: () => void;
  onConsolidateAllExceptConsolidated: () => void;
  /** Toggle one sector into or out of the target. The menu stays open. */
  onToggleSector: (sector: string) => void;
  onDeconsolidateAllFromTarget: () => void;
  onDeconsolidateAllInCenter: () => void;
  /** Release one sector back to its own row. The menu stays open. */
  onReleaseSector: (sector: string) => void;
  onClose: () => void;
};

export function SectorContextMenu({
  x,
  y,
  target,
  center,
  sectors,
  consolidatedHere,
  hasAnyConsolidation,
  canMoveUp,
  canMoveDown,
  onMoveUp,
  onMoveDown,
  onConsolidateAll,
  onConsolidateAllExceptConsolidated,
  onToggleSector,
  onDeconsolidateAllFromTarget,
  onDeconsolidateAllInCenter,
  onReleaseSector,
  onClose,
}: SectorContextMenuProps) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const [openL2, setOpenL2] = useState<Branch | null>(null);
  const [openL3, setOpenL3] = useState(false);
  const [hot1, setHot1] = useState<number | null>(null);
  const [hot2, setHot2] = useState<number | null>(null);
  const [hot3, setHot3] = useState<number | null>(null);
  const anyHere = consolidatedHere.length > 0;

  const clearTimer = () => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  };
  useEffect(() => () => clearTimer(), []);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (e.target instanceof Node && rootRef.current?.contains(e.target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      if (openL3) setOpenL3(false);
      else if (openL2) setOpenL2(null);
      else onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [openL2, openL3, onClose]);

  /** Entering a level-1 row: a branch opens after a beat; anything else retracts the subtree at once. */
  const enterL1 = (i: number, branch: Branch | null) => {
    setHot1(i);
    clearTimer();
    if (!branch) {
      setOpenL2(null);
      setOpenL3(false);
      return;
    }
    if (openL2 === branch) return;
    setOpenL2(null);
    setOpenL3(false);
    setHot2(null);
    timer.current = setTimeout(() => setOpenL2(branch), MENU.submenuOpenMs);
  };
  const enterL2 = (i: number, isList: boolean) => {
    setHot2(i);
    clearTimer();
    if (isList) {
      if (!openL3) timer.current = setTimeout(() => setOpenL3(true), MENU.submenuOpenMs);
    } else {
      setOpenL3(false);
    }
  };
  const openBranchNow = (branch: Branch) => {
    clearTimer();
    setOpenL2(branch);
    setOpenL3(false);
    setHot2(null);
  };
  const run = (fn: () => void) => {
    fn();
    onClose();
  };

  return createPortal(
    <div ref={rootRef} data-sector-menu-root="">
      <RootPanel x={x} y={y}>
        <Row label="Move Row Up" hovered={hot1 === 0} disabled={!canMoveUp} onEnter={() => enterL1(0, null)} onClick={() => run(onMoveUp)} />
        <Row label="Move Row Down" hovered={hot1 === 1} disabled={!canMoveDown} onEnter={() => enterL1(1, null)} onClick={() => run(onMoveDown)} />
        <Row
          label="Consolidate"
          arrow
          hovered={hot1 === 2 || openL2 === "consolidate"}
          onEnter={() => enterL1(2, "consolidate")}
          onClick={() => openBranchNow("consolidate")}
        >
          {openL2 === "consolidate" && (
            <Submenu>
              <Row label={`Consolidate All into ${target}`} hovered={hot2 === 0} onEnter={() => enterL2(0, false)} onClick={() => run(onConsolidateAll)} />
              <Row
                label={`Consolidate All into ${target} Except Consolidated`}
                hovered={hot2 === 1}
                onEnter={() => enterL2(1, false)}
                onClick={() => run(onConsolidateAllExceptConsolidated)}
              />
              <Row
                label={`Consolidate into ${target}`}
                arrow
                hovered={hot2 === 2 || openL3}
                onEnter={() => enterL2(2, true)}
                onClick={() => {
                  clearTimer();
                  setOpenL3(true);
                }}
              >
                {openL3 && (
                  <Submenu scroll>
                    {sectors.length === 0 ? (
                      <Row label="(no other sectors)" hovered={false} disabled onEnter={() => {}} onClick={() => {}} />
                    ) : (
                      sectors.map((s, i) => (
                        <Row
                          key={s.sector}
                          label={s.sector}
                          checkColumn
                          checked={s.checked}
                          hovered={hot3 === i}
                          onEnter={() => {
                            clearTimer();
                            setHot3(i);
                          }}
                          onClick={() => onToggleSector(s.sector)}
                        />
                      ))
                    )}
                  </Submenu>
                )}
              </Row>
            </Submenu>
          )}
        </Row>
        <Row
          label="Deconsolidate"
          arrow
          hovered={hot1 === 3 || openL2 === "deconsolidate"}
          disabled={!hasAnyConsolidation}
          onEnter={() => enterL1(3, hasAnyConsolidation ? "deconsolidate" : null)}
          onClick={() => openBranchNow("deconsolidate")}
        >
          {openL2 === "deconsolidate" && (
            <Submenu>
              <Row
                label={`Deconsolidate All from ${target}`}
                hovered={hot2 === 0}
                disabled={!anyHere}
                onEnter={() => enterL2(0, false)}
                onClick={() => run(onDeconsolidateAllFromTarget)}
              />
              <Row
                label={`Deconsolidate All in ${center}`}
                hovered={hot2 === 1}
                onEnter={() => enterL2(1, false)}
                onClick={() => run(onDeconsolidateAllInCenter)}
              />
              <Row
                label={`Deconsolidate from ${target}`}
                arrow
                hovered={hot2 === 2 || openL3}
                disabled={!anyHere}
                onEnter={() => enterL2(2, anyHere)}
                onClick={() => {
                  clearTimer();
                  setOpenL3(true);
                }}
              >
                {openL3 && anyHere && (
                  <Submenu scroll>
                    {consolidatedHere.map((s, i) => (
                      <Row
                        key={s.sector}
                        label={s.sector}
                        checkColumn
                        checked={s.checked}
                        hovered={hot3 === i}
                        onEnter={() => {
                          clearTimer();
                          setHot3(i);
                        }}
                        onClick={() => onReleaseSector(s.sector)}
                      />
                    ))}
                  </Submenu>
                )}
              </Row>
            </Submenu>
          )}
        </Row>
      </RootPanel>
    </div>,
    document.body,
  );
}
