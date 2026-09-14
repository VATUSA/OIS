import {useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";

import {AadcChart} from "@/components/aadc-chart";
import {type AadcBucketMin, type AadcDimension, useAadc} from "@/lib/aadc";

export function AadcPage() {
  const [query, setQuery] = useState("");
  const [icao, setIcao] = useState("");
  const [bucketMin, setBucketMin] = useState<AadcBucketMin>(15);
  const [dimension, setDimension] = useState<AadcDimension>("status");
  const aadc = useAadc(icao || null, bucketMin);

  function load() {
    const clean = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (clean.length >= 3) setIcao(clean);
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Arrival demand chart</h1>
        <p className="text-muted-foreground">
          Forward arrival demand for an airport, bucketed by time and broken down by status,
          aircraft category, carrier, or arrival fix — against the wind-favored AAR.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-wrap items-end gap-3 pt-6">
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Airport
            <Input
              className="w-32 font-mono uppercase"
              maxLength={4}
              placeholder="KJFK"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && load()}
            />
          </label>
          <Button onClick={load}>Load</Button>
          {icao && aadc.data && (
            <span className="ml-auto text-xs text-muted-foreground">
              {aadc.isFetching ? "refreshing…" : "live · updates every 20s"}
            </span>
          )}
        </CardContent>
      </Card>

      {!icao ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Enter an arrival airport above to see its demand chart.
          </CardContent>
        </Card>
      ) : aadc.isError ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Couldn&apos;t load AADC data for {icao}.
          </CardContent>
        </Card>
      ) : !aadc.data ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Loading {icao}…
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="pt-6">
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
          </CardContent>
        </Card>
      )}
    </div>
  );
}
