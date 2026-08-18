// The historical-mode context now lives in lib/ (so lib query hooks can read it). Re-exported here
// for the dashboard feature's existing import sites.
export {
  HistoricalProvider,
  useHistorical,
  useHistoricalAt,
  type HistoricalWindow,
} from "@/lib/historical-context";
