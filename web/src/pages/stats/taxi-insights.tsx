import {useEffect, useState} from "react";
import {Badge, Button, Card, CardContent, Input, Switch} from "@ois/ui";

import {Pagination} from "@/components/pagination";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  useTaxiEstimates,
  useTaxiObservations,
  type TaxiInsightsFilters,
} from "@/lib/taxi-insights";
import {formatZuluFull} from "@/lib/time";

const PAGE_SIZE = 50;

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const selectClass =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

const TIERS = [
  { value: "", label: "Any tier" },
  { value: "gate_type_runway", label: "Gate + type + runway" },
  { value: "airport_runway", label: "Airport + runway" },
  { value: "airport", label: "Airport-wide" },
  // No duration in this label: the default is a different flat value per metric (pushback,
  // start-up, taxi), and the actual value is already shown alongside the badge — a single
  // hardcoded duration here would be wrong for the others.
  { value: "default", label: "Default" },
];

/** Seconds → "Mm Ss", or "—" when unknown. Rounds to the nearest whole second first so a
 * fractional remainder (e.g. 299.5) can't round up to "60s" instead of carrying into the minute. */
export function fmtDur(sec: number | null | undefined): string {
  if (sec == null) return "—";
  const total = Math.round(sec);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}m ${s}s`;
}

function tierLabel(tier: string): string {
  return TIERS.find((t) => t.value === tier)?.label ?? tier;
}

export function TaxiInsightsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const [tab, setTab] = useState<"observations" | "estimates">("observations");
  const [page, setPage] = useState(1);

  const [airportDraft, setAirportDraft] = useState("");
  const [gateDraft, setGateDraft] = useState("");
  const [aircraftDraft, setAircraftDraft] = useState("");
  const [runwayDraft, setRunwayDraft] = useState("");
  const [fromDraft, setFromDraft] = useState("");
  const [toDraft, setToDraft] = useState("");
  const [includeOutliers, setIncludeOutliers] = useState(true);
  const [fallbackTier, setFallbackTier] = useState("");

  const [filters, setFilters] = useState<TaxiInsightsFilters>({});

  useEffect(() => {
    setPage(1);
  }, [tab, filters, fallbackTier]);

  const apply = () =>
    setFilters({
      airport: normIcao(airportDraft) || undefined,
      gateId: gateDraft.trim() || undefined,
      aircraft: aircraftDraft.trim().toUpperCase() || undefined,
      runway: runwayDraft.trim().toUpperCase() || undefined,
      // datetime-local yields "YYYY-MM-DDTHH:mm"; append seconds so it parses as RFC 3339.
      from: fromDraft ? `${fromDraft}:00Z` : undefined,
      to: toDraft ? `${toDraft}:00Z` : undefined,
      includeOutliers,
    });
  const clear = () => {
    setAirportDraft("");
    setGateDraft("");
    setAircraftDraft("");
    setRunwayDraft("");
    setFromDraft("");
    setToDraft("");
    setIncludeOutliers(true);
    setFallbackTier("");
    setFilters({});
  };
  const hasFilters = Boolean(
    filters.airport ||
      filters.gateId ||
      filters.aircraft ||
      filters.runway ||
      filters.from ||
      filters.to ||
      !filters.includeOutliers,
  );

  const observations = useTaxiObservations(page, PAGE_SIZE, filters);
  const estimates = useTaxiEstimates(page, PAGE_SIZE, { ...filters, fallbackTier });

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-1">
        <h1 className="text-2xl font-semibold tracking-tight">Taxi &amp; Pushback Insights</h1>
        <p className="text-muted-foreground">
          Raw departure timing observations and their learned per-gate/type/runway estimates.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <div className="flex overflow-hidden rounded-md border">
            {(["observations", "estimates"] as const).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTab(t)}
                className={
                  "px-3 py-1.5 text-sm font-medium transition-colors " +
                  (tab === t ? "bg-primary text-primary-foreground" : "hover:bg-accent/40")
                }
              >
                {t === "observations" ? "Observations" : "Estimates"}
              </button>
            ))}
          </div>

          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              apply();
            }}
          >
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              Airport {tab === "estimates" && "(required)"}
              <Input
                placeholder="KJFK"
                value={airportDraft}
                onChange={(e) => setAirportDraft(e.target.value)}
                className="w-24"
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              Gate
              <Input
                placeholder="Gate id"
                value={gateDraft}
                onChange={(e) => setGateDraft(e.target.value)}
                className="w-28"
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              Aircraft
              <Input
                placeholder="B738"
                value={aircraftDraft}
                onChange={(e) => setAircraftDraft(e.target.value)}
                className="w-24"
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              Runway
              <Input
                placeholder="27L"
                value={runwayDraft}
                onChange={(e) => setRunwayDraft(e.target.value)}
                className="w-20"
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              From
              <Input
                type="datetime-local"
                value={fromDraft}
                onChange={(e) => setFromDraft(e.target.value)}
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              To
              <Input
                type="datetime-local"
                value={toDraft}
                onChange={(e) => setToDraft(e.target.value)}
              />
            </label>
            {tab === "estimates" && (
              <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
                Fallback tier
                <select
                  value={fallbackTier}
                  onChange={(e) => setFallbackTier(e.target.value)}
                  className={selectClass}
                >
                  {TIERS.map((t) => (
                    <option key={t.value} value={t.value}>
                      {t.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <label className="flex items-center gap-1.5 pb-1.5 text-xs text-muted-foreground">
              <Switch checked={includeOutliers} onCheckedChange={setIncludeOutliers} className="scale-[0.68]" />
              Include outliers
            </label>
            <Button type="submit" size="sm">
              Apply
            </Button>
            {hasFilters && (
              <Button type="button" size="sm" variant="ghost" onClick={clear}>
                Clear
              </Button>
            )}
          </form>

          {tab === "observations" ? (
            observations.data ? (
              <>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <th className="py-2 pr-3">Time</th>
                        <th className="py-2 pr-3">Airport</th>
                        <th className="py-2 pr-3">Gate</th>
                        <th className="py-2 pr-3">Aircraft</th>
                        <th className="py-2 pr-3">Runway</th>
                        <th className="py-2 pr-3">Pushback</th>
                        <th className="py-2 pr-3">Start-up</th>
                        <th className="py-2 pr-3">Taxi</th>
                        <th className="py-2 pr-3">Outlier</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-border/60">
                      {observations.data.items.map((o) => (
                        <tr key={o.id}>
                          <td className="py-2 pr-3 whitespace-nowrap">{formatZuluFull(o.observed_at)}</td>
                          <td className="py-2 pr-3 font-mono font-semibold">{o.airport}</td>
                          <td className="py-2 pr-3 font-mono text-xs">{o.gate_id ?? "—"}</td>
                          <td className="py-2 pr-3">{o.aircraft ?? "—"}</td>
                          <td className="py-2 pr-3">{o.runway ?? "—"}</td>
                          <td className="py-2 pr-3">{fmtDur(o.pushback_sec)}</td>
                          <td className="py-2 pr-3">{fmtDur(o.startup_sec)}</td>
                          <td className="py-2 pr-3">{fmtDur(o.taxi_sec)}</td>
                          <td className="py-2 pr-3">
                            {o.is_outlier && <Badge variant="destructive">outlier</Badge>}
                          </td>
                        </tr>
                      ))}
                      {observations.data.items.length === 0 && (
                        <tr>
                          <td colSpan={9} className="py-6 text-center text-muted-foreground">
                            No observations match these filters.
                          </td>
                        </tr>
                      )}
                    </tbody>
                  </table>
                </div>
                <Pagination
                  page={observations.data.page}
                  pageSize={observations.data.page_size}
                  total={observations.data.total}
                  onPageChange={setPage}
                />
              </>
            ) : observations.isError ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                Couldn&apos;t load taxi observations.
              </p>
            ) : (
              <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
            )
          ) : !filters.airport ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Enter an airport above — estimates are computed per airport.
            </p>
          ) : estimates.data ? (
            <>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b text-left text-xs uppercase tracking-wide text-muted-foreground">
                      <th className="py-2 pr-3">Airport</th>
                      <th className="py-2 pr-3">Gate</th>
                      <th className="py-2 pr-3">Aircraft</th>
                      <th className="py-2 pr-3">Runway</th>
                      <th className="py-2 pr-3">Pushback</th>
                      <th className="py-2 pr-3">Start-up</th>
                      <th className="py-2 pr-3">Taxi</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border/60">
                    {estimates.data.items.map((e, i) => (
                      <tr key={i}>
                        <td className="py-2 pr-3 font-mono font-semibold">{e.airport}</td>
                        <td className="py-2 pr-3 font-mono text-xs">{e.gate_id ?? "—"}</td>
                        <td className="py-2 pr-3">{e.aircraft ?? "—"}</td>
                        <td className="py-2 pr-3">{e.runway ?? "—"}</td>
                        <td className="py-2 pr-3">
                          <div className="flex items-center gap-1.5">
                            {fmtDur(e.pushback_sec)}
                            <Badge variant="secondary">{tierLabel(e.pushback_tier)}</Badge>
                            <span className="text-xs text-muted-foreground">
                              n={e.pushback_sample_count}
                            </span>
                          </div>
                        </td>
                        <td className="py-2 pr-3">
                          <div className="flex items-center gap-1.5">
                            {fmtDur(e.startup_sec)}
                            <Badge variant="secondary">{tierLabel(e.startup_tier)}</Badge>
                            <span className="text-xs text-muted-foreground">
                              n={e.startup_sample_count}
                            </span>
                          </div>
                        </td>
                        <td className="py-2 pr-3">
                          <div className="flex items-center gap-1.5">
                            {fmtDur(e.taxi_sec)}
                            <Badge variant="secondary">{tierLabel(e.taxi_tier)}</Badge>
                            <span className="text-xs text-muted-foreground">
                              n={e.taxi_sample_count}
                            </span>
                          </div>
                        </td>
                      </tr>
                    ))}
                    {estimates.data.items.length === 0 && (
                      <tr>
                        <td colSpan={7} className="py-6 text-center text-muted-foreground">
                          No estimates match these filters.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
              <Pagination
                page={estimates.data.page}
                pageSize={estimates.data.page_size}
                total={estimates.data.total}
                onPageChange={setPage}
              />
            </>
          ) : estimates.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load taxi estimates.
            </p>
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
