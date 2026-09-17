import type {PickingInfo} from "@deck.gl/core";

/** The slice of deck's instance we need to look under the topmost pick. */
interface DeckPicker {
  pickObject(opts: { x: number; y: number; radius?: number; layerIds?: string[] }): PickingInfo | null;
}

/**
 * The FCA whose line lies under `info`, or `null`. deck hands a click only the topmost layer, and
 * the invisible `atc-hover` target sits above `fca-lines` so an ATC pill wins the hover card — so a
 * click that lands on a pill has to look through it for the line it covers (#323).
 */
export function fcaLineUnder(info: PickingInfo): string | null {
  const deck = (info.layer?.context as { deck?: DeckPicker } | undefined)?.deck;
  if (!deck) return null;
  const under = deck.pickObject({ x: info.x, y: info.y, radius: 4, layerIds: ["fca-lines"] });
  return (under?.object as { id?: string } | undefined)?.id ?? null;
}
