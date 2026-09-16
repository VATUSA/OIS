import {useState} from "react";
import {Button, Card, EmptyState, Input, QueryState} from "@ois/ui";
import {BarChart3} from "lucide-react";

import {AadcChart} from "@/components/aadc-chart";
import {usePageHeader} from "@/components/shell/page-meta";
import {type AadcBucketMin, type AadcDimension, useAadc} from "@/lib/aadc";

const SUBTITLE =
  "Forward arrival demand for an airport, bucketed by time and broken down by status, aircraft category, carrier, or arrival fix — against the wind-favored AAR.";

export function AadcPage() {
  const [query, setQuery] = useState("");
  const [icao, setIcao] = useState("");
  const [bucketMin, setBucketMin] = useState<AadcBucketMin>(15);
  const [dimension, setDimension] = useState<AadcDimension>("status");
  const aadc = useAadc(icao || null, bucketMin);

  usePageHeader({ subtitle: SUBTITLE });

  function load() {
    const clean = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (clean.length >= 3) setIcao(clean);
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          aria-label="Airport"
          className="w-28 font-mono uppercase"
          maxLength={4}
          placeholder="KJFK"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && load()}
        />
        <Button onClick={load}>Load</Button>
        {icao && aadc.data && (
          <span className="ml-auto text-xs text-ink-3">
            {aadc.isFetching ? "refreshing…" : "live · updates every 20s"}
          </span>
        )}
      </div>

      {!icao ? (
        <EmptyState icon={BarChart3} className="rounded-md border border-line py-12">
          Enter an arrival airport above to see its demand chart.
        </EmptyState>
      ) : (
        <QueryState
          isLoading={!aadc.data && !aadc.isError}
          isError={aadc.isError}
          loading={`Loading ${icao}…`}
          error={`Couldn't load AADC data for ${icao}.`}
          onRetry={() => aadc.refetch()}
          className="rounded-md border border-line py-12"
        >
          {aadc.data && (
            <Card className="p-4">
              <AadcChart
                icao={icao}
                bucketMin={bucketMin}
                onBucketMinChange={setBucketMin}
                dimension={dimension}
                onDimensionChange={setDimension}
                buckets={aadc.data.buckets}
                aar={aadc.data.aar}
                isLoading={aadc.isFetching}
              />
            </Card>
          )}
        </QueryState>
      )}
    </div>
  );
}
