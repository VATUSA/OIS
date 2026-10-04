import {useMemo} from "react";
import {QueryState, cn} from "@ois/ui";

import {usePageHeader} from "@/components/shell/page-meta";
import {MapCanvas} from "@/components/map/MapCanvas";
import {useMapCamera} from "@/components/map/hooks/useMapCamera";
import {useMapPalette} from "@/components/map/lib/colors";
import {MAP_BUTTON, MAP_BUTTON_ON, MAP_PANEL} from "@/components/map/lib/overlay";
import {sectorTooltip} from "@/components/map/lib/tooltip";
import {SECTOR_TIERS, buildSectorLayer} from "@/components/map/layers/sectors";
import {useSectors} from "@/lib/sectors";
import {useSetting} from "@/lib/settings";

/** High sectors first: they tile the country without overlapping, so they read on their own. */
const DEFAULT_STRATA: string[] = ["high"];

/**
 * Admin sector map (#602): every ATC sector volume (#594), one stratum at a time, with a hover card.
 * Internal monitoring data, so it lives only here, behind `flow.sectors.read` (the nav item's
 * permission is the page gate).
 */
export function SectorsPage() {
  const sectors = useSectors();
  const palette = useMapPalette();
  const camera = useMapCamera(undefined, {
    persistKey: "sectors",
    persist: useSetting("map.persistView", true).value,
  });
  const { value: strata, setValue: setStrata } = useSetting<string[]>("sectors.strata", DEFAULT_STRATA);

  usePageHeader({
    subtitle: "ATC sector volumes, one stratum at a time. Hover a sector for its name and vertical band.",
    count: sectors.data?.length ?? null,
  });

  const layers = useMemo(
    () => (sectors.data ? [buildSectorLayer(sectors.data, strata, palette)] : []),
    [sectors.data, strata, palette],
  );
  const toggle = (tier: string) =>
    setStrata(strata.includes(tier) ? strata.filter((t) => t !== tier) : [...strata, tier]);

  return (
    <QueryState
      isLoading={sectors.isLoading}
      isError={sectors.isError}
      onRetry={() => sectors.refetch()}
      isEmpty={sectors.data?.length === 0}
      empty="No sector volumes are loaded. Run the airspace-sector-importer; the map picks them up within five minutes."
      className="rounded-md border border-line"
    >
        <div className="relative isolate h-[75vh] w-full overflow-hidden rounded-md border border-line">
          <MapCanvas
            viewState={camera.viewState}
            onViewStateChange={camera.onViewStateChange}
            onResize={camera.onResize}
            controller
            layers={layers}
            getTooltip={sectorTooltip()}
          >
            <div className="absolute left-3 top-3 z-[500] flex flex-wrap items-center gap-2">
              {SECTOR_TIERS.map(({ tier, label }) => {
                const on = strata.includes(tier);
                return (
                  <button
                    key={tier}
                    type="button"
                    aria-pressed={on}
                    onClick={() => toggle(tier)}
                    className={cn(MAP_BUTTON, on && MAP_BUTTON_ON)}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
            <ul className={cn(MAP_PANEL, "absolute bottom-3 left-3 z-[500] flex flex-col gap-1 px-3 py-2 text-xs")}>
              {SECTOR_TIERS.map(({ tier, label, series }) => (
                <li key={tier} className="flex items-center gap-2 text-ink-2">
                  <span
                    aria-hidden
                    className="size-2.5 rounded-sm"
                    style={{ background: `var(--series-${series + 1})` }}
                  />
                  {label}
                </li>
              ))}
            </ul>
          </MapCanvas>
        </div>
    </QueryState>
  );
}
