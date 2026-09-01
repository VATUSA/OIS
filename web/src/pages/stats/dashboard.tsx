import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import {Badge, Button, Card, CardContent, useToast} from "@ois/ui";
import {useNavigate, useSearch} from "@tanstack/react-router";
import {Link2, Pause, Play, Rewind} from "lucide-react";

import {ZuluDateTime} from "@/components/zulu-datetime";
import {DashboardGrid} from "@/features/dashboard/DashboardGrid";
import {HistoricalProvider} from "@/features/dashboard/historical";
import {EMPTY_DASHBOARD, type DashboardState} from "@/features/dashboard/types";
import {useMe} from "@/lib/auth";
import {useDashboard, useDashboards} from "@/lib/dashboards";
import {hasPermission} from "@/lib/permissions";
import {useCaptures} from "@/lib/stats";
import {formatZuluFull} from "@/lib/time";

/** Deep-link search params for a shareable replay (validated on the route). */
interface DashboardSearch {
  capture?: string;
  from?: number;
  to?: number;
  board?: string;
  t?: number;
}

/** Committed scrubber instants snap to this many seconds — fewer distinct `at` values means the
 * per-instant reconstructions cache and revisiting a time is instant. */
const SNAP_S = 30;
const SPEEDS = [1, 2, 4, 8, 16] as const;
/** Real-time ms between play steps. */
const TICK_MS = 700;

/** Coerce a stored board blob into a valid DashboardState (mirrors useDashboardState.normalize). */
function normalize(raw: unknown): DashboardState {
  const s = raw as DashboardState | null | undefined;
  if (!s || s.version !== 1 || !Array.isArray(s.widgets) || !Array.isArray(s.layout)) {
    return EMPTY_DASHBOARD;
  }
  return s;
}

const zulu = (unixS: number) =>
  new Date(unixS * 1000).toISOString().slice(11, 16) + "Z";


interface Win {
  from: number;
  to: number;
}

/** Copies the current URL (kept in sync with the pickers + scrubber) so a replay can be shared. */
function CopyLink() {
  const toast = useToast();
  return (
    <Button
      size="sm"
      variant="ghost"
      title="Copy a link to this replay"
      onClick={() => {
        void navigator.clipboard?.writeText(window.location.href).then(
          () => toast.success("Link copied"),
          () => toast.error("Couldn’t copy the link"),
        );
      }}
    >
      <Link2 className="size-4" />
      Share
    </Button>
  );
}

/** The scrubber + read-only board render for a chosen [from, to] window. `initialT` seeds the
 * scrubber from a shared link; `onCommit` reports the (snapped, debounced) instant back so the page
 * can keep it in the URL. */
function Replay({
  win,
  state,
  initialT,
  onCommit,
}: {
  win: Win;
  state: DashboardState;
  initialT?: number;
  onCommit?: (t: number) => void;
}) {
  const span = Math.max(1, win.to - win.from);
  const mid = win.from + Math.floor(span / 2);
  const [scrub, setScrub] = useState(() =>
    initialT != null && initialT >= win.from && initialT <= win.to ? initialT : mid,
  );
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState<number>(4);

  // Reset to the middle when the window actually changes — but not on first mount, so a deep link's
  // `initialT` survives.
  const winKey = `${win.from}-${win.to}`;
  const prevWinKey = useRef(winKey);
  useEffect(() => {
    if (prevWinKey.current === winKey) return;
    prevWinKey.current = winKey;
    setScrub(win.from + Math.floor(span / 2));
    setPlaying(false);
  }, [winKey, win.from, span]);

  // Playback: advance the scrubber in real time; stop at the end.
  const scrubRef = useRef(scrub);
  scrubRef.current = scrub;
  useEffect(() => {
    if (!playing) return;
    const id = window.setInterval(() => {
      const next = scrubRef.current + SNAP_S * speed;
      if (next >= win.to) {
        setScrub(win.to);
        setPlaying(false);
      } else {
        setScrub(next);
      }
    }, TICK_MS);
    return () => window.clearInterval(id);
  }, [playing, speed, win.to]);

  // Snap the committed instant so reconstructions cache; debounce so a fast drag doesn't stampede.
  const snapped = Math.round(scrub / SNAP_S) * SNAP_S;
  const [committed, setCommitted] = useState(snapped);
  useEffect(() => {
    const id = window.setTimeout(() => setCommitted(snapped), 120);
    return () => window.clearTimeout(id);
  }, [snapped]);

  // Report the committed instant up (for the shareable URL) via a ref so a changing `onCommit`
  // identity doesn't re-fire the effect.
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;
  useEffect(() => {
    onCommitRef.current?.(committed);
  }, [committed]);

  const pct = ((scrub - win.from) / span) * 100;

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardContent className="flex flex-col gap-3 pt-6">
          <div className="flex items-center gap-3">
            <Button
              size="sm"
              variant={playing ? "default" : "secondary"}
              onClick={() => setPlaying((p) => !p)}
            >
              {playing ? <Pause className="size-4" /> : <Play className="size-4" />}
              {playing ? "Pause" : "Play"}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setPlaying(false);
                setScrub(win.from);
              }}
              title="Back to start"
            >
              <Rewind className="size-4" />
            </Button>
            <div className="flex items-center gap-1">
              {SPEEDS.map((s) => (
                <Button
                  key={s}
                  size="sm"
                  variant={speed === s ? "default" : "secondary"}
                  className="h-7 px-2 tabular-nums"
                  onClick={() => setSpeed(s)}
                >
                  {s}×
                </Button>
              ))}
            </div>
            <div className="ml-auto flex items-center gap-3">
              <CopyLink />
              <span className="font-mono text-lg tabular-nums">{zulu(scrub)}</span>
            </div>
          </div>
          <input
            type="range"
            min={win.from}
            max={win.to}
            step={SNAP_S}
            value={scrub}
            onChange={(e) => {
              setPlaying(false);
              setScrub(Number(e.target.value));
            }}
            className="w-full accent-primary"
            style={{ background: `linear-gradient(to right, var(--primary) ${pct}%, var(--border) ${pct}%)` }}
          />
          <div className="flex justify-between text-[11px] text-muted-foreground">
            <span>{formatZuluFull(new Date(win.from * 1000).toISOString())}</span>
            <span>{formatZuluFull(new Date(win.to * 1000).toISOString())}</span>
          </div>
        </CardContent>
      </Card>

      {state.widgets.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            This board has no widgets. Add table or chart widgets on it (arrivals, departures,
            traffic) — those replay historically.
          </CardContent>
        </Card>
      ) : (
        <HistoricalProvider value={{ from: win.from, to: win.to, t: committed }}>
          <DashboardGrid
            state={state}
            editing={false}
            onLayoutChange={() => {}}
            onRemove={() => {}}
            onUpdate={() => {}}
          />
        </HistoricalProvider>
      )}
    </div>
  );
}

export function HistoricalDashboardPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const captures = useCaptures();
  const boards = useDashboards();

  // The current selection lives in the URL, so a replay is a shareable link (validated on the
  // route). State is seeded from the URL once; the handlers below keep both in sync.
  const search = useSearch({ strict: false }) as DashboardSearch;
  const navigate = useNavigate();
  const patchSearch = useCallback(
    (patch: Partial<DashboardSearch>) => {
      void navigate({
        to: "/historical/dashboard",
        search: (prev) => ({ ...prev, ...patch }),
        replace: true,
        // Keep the URL in sync as the scrubber advances without yanking the page back to the top.
        resetScroll: false,
      });
    },
    [navigate],
  );

  const [captureId, setCaptureId] = useState<string>(() => search.capture ?? "");
  // Default custom window = the last 3 hours, active immediately (so the board renders without
  // forcing an edit first). Selecting a capture takes over; switching back restores a default.
  const defaultWindow = (): Win => {
    const now = Math.floor(Date.now() / 1000);
    return { from: now - 3 * 3600, to: now };
  };
  const [custom, setCustom] = useState<Win>(() =>
    search.from != null && search.to != null && search.to > search.from
      ? { from: search.from, to: search.to }
      : defaultWindow(),
  );
  const [boardId, setBoardId] = useState<string>(() => search.board ?? "");

  const board = useDashboard(boardId || null);
  const state = useMemo(
    () => (board.data ? normalize(board.data.data) : null),
    [board.data],
  );

  // The active window: a selected capture, or the custom [from, to].
  const win: Win | null = useMemo(() => {
    if (captureId) {
      const c = captures.data?.find((x) => x.id === captureId);
      if (!c) return null;
      const from = Math.floor(Date.parse(c.start_time) / 1000);
      const to = Math.floor(Date.parse(c.end_time ?? new Date().toISOString()) / 1000);
      return to > from ? { from, to } : null;
    }
    return custom.to > custom.from ? custom : null;
  }, [captureId, captures.data, custom]);

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </CardContent>
      </Card>
    );
  }

  const boardList = boards.data?.dashboards ?? [];

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Historical dashboard</h1>
        <p className="text-muted-foreground">
          Replay one of your dashboards over a past event capture or time window — the same widgets,
          recomputed at a scrubber instant.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <div className="grid gap-4 md:grid-cols-2">
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted-foreground">Capture window</span>
              <select
                className="h-9 rounded-md border bg-background px-2"
                value={captureId}
                onChange={(e) => {
                  const v = e.target.value;
                  setCaptureId(v);
                  if (v) {
                    patchSearch({ capture: v, from: undefined, to: undefined, t: undefined });
                  } else {
                    const w = defaultWindow();
                    setCustom(w);
                    patchSearch({ capture: undefined, from: w.from, to: w.to, t: undefined });
                  }
                }}
              >
                <option value="">— custom time window —</option>
                {(captures.data ?? []).map((c) => (
                  <option key={c.id} value={c.id}>
                    {(c.event_title || c.label || "Capture") +
                      ` · ${formatZuluFull(c.start_time)}` +
                      (c.status === "open" ? " (recording)" : "")}
                  </option>
                ))}
              </select>
            </label>

            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted-foreground">Board</span>
              <select
                className="h-9 rounded-md border bg-background px-2"
                value={boardId}
                onChange={(e) => {
                  setBoardId(e.target.value);
                  patchSearch({ board: e.target.value || undefined });
                }}
              >
                <option value="">— select a board —</option>
                {boardList.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {!captureId && (
            <div className="flex flex-wrap items-end gap-3">
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">From (Zulu)</span>
                <ZuluDateTime
                  value={custom.from}
                  onChange={(from) => {
                    setCustom((c) => ({ from, to: c.to }));
                    patchSearch({ from, to: custom.to, capture: undefined, t: undefined });
                  }}
                />
              </label>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">To (Zulu)</span>
                <ZuluDateTime
                  value={custom.to}
                  onChange={(to) => {
                    setCustom((c) => ({ from: c.from, to }));
                    patchSearch({ from: custom.from, to, capture: undefined, t: undefined });
                  }}
                />
              </label>
              <Badge variant="outline">retained ~14 days</Badge>
            </div>
          )}
        </CardContent>
      </Card>

      {!win ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Pick a capture (or a custom window) to begin.
          </CardContent>
        </Card>
      ) : !boardId ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Select one of your boards to replay over this window.
          </CardContent>
        </Card>
      ) : !state ? (
        <p className="py-12 text-center text-sm text-muted-foreground">Loading board…</p>
      ) : (
        <Replay
          win={win}
          state={state}
          initialT={search.t}
          onCommit={(t) => patchSearch({ t })}
        />
      )}
    </div>
  );
}
