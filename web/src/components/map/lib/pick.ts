import type {PickingInfo} from "@deck.gl/core";

/** The slice of deck's instance we need to look under the topmost pick. */
interface DeckPicker {
  pickObject(opts: { x: number; y: number; radius?: number; layerIds?: string[] }): PickingInfo | null;
}

/**
 * The topmost object of `layerId` at `info`'s pixel, looking past whatever actually won the pick.
 * deck reports only the topmost layer, so a handler that wants what sits *underneath* has to ask
 * again, restricted to the layer it cares about (#323). `null` when nothing is there, or when the
 * deck instance isn't reachable from the pick (as in unit tests).
 */
export function objectUnder(info: PickingInfo, layerId: string): unknown {
  const deck = (info.layer?.context as { deck?: DeckPicker } | undefined)?.deck;
  if (!deck) return null;
  const under = deck.pickObject({ x: info.x, y: info.y, radius: 4, layerIds: [layerId] });
  return under?.object ?? null;
}

/**
 * The FCA whose line lies under `info`, or `null`. The invisible `atc-hover` target sits above
 * `fca-lines` so an ATC pill wins the hover card — so a click that lands on a pill has to look
 * through it for the line it covers (#323).
 */
export function fcaLineUnder(info: PickingInfo): string | null {
  return (objectUnder(info, "fca-lines") as { id?: string } | null)?.id ?? null;
}
