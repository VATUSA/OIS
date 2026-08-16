import {Card, CardContent} from "@ois/ui";

import {AircraftView, DemandView, LadderView, SummaryView} from "@/pages/airport";
import {DeparturesView} from "@/pages/departures";
import {TaxiView} from "@/pages/taxi";
import {useAirportFlow} from "@/lib/feed";

import type {ViewId} from "./types";

/** Views that need only an ICAO and self-fetch. */
const AIRPORT_VIEWS = {
  "airport-summary": SummaryView,
  "airport-aircraft": AircraftView,
  "airport-ladder": LadderView,
  "airport-demand": DemandView,
} as const;

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <Card>
      <CardContent className="py-8 text-center text-sm text-muted-foreground">
        {children}
      </CardContent>
    </Card>
  );
}

/** The four airport views take a pre-fetched `flow`; fetch it here from the widget's ICAO. */
function AirportFlowView({ icao, view }: { icao: string; view: keyof typeof AIRPORT_VIEWS }) {
  const flow = useAirportFlow(icao);
  const View = AIRPORT_VIEWS[view];
  if (flow.isError) return <Notice>Couldn&apos;t load {icao}.</Notice>;
  if (!flow.data) return <Notice>Loading {icao}…</Notice>;
  return <View flow={flow.data} />;
}

/** Views available as widgets, and their labels for the add-widget menu. */
export const VIEW_OPTIONS: { id: ViewId; label: string }[] = [
  { id: "airport-summary", label: "Airport — Summary" },
  { id: "airport-aircraft", label: "Airport — Aircraft list" },
  { id: "airport-ladder", label: "Airport — Arrival ladder" },
  { id: "airport-demand", label: "Airport — Demand vs AAR" },
  { id: "departures", label: "Departures" },
  { id: "taxi", label: "Taxi times" },
];

export function ViewWidgetView({ view, icao }: { view: ViewId; icao: string }) {
  switch (view) {
    case "departures":
      return <DeparturesView icao={icao} />;
    case "taxi":
      return <TaxiView icao={icao} />;
    default:
      return <AirportFlowView icao={icao} view={view} />;
  }
}
