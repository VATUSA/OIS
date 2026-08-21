import {describe, expect, it} from "vitest";

import {buildFcaLayers, type MapFca} from "./fca";

const fca = (over: Partial<MapFca>): MapFca => ({
  id: "a",
  name: "A",
  color: "#ff0000",
  enabled: true,
  points: [
    [40, -80],
    [41, -79],
  ],
  ...over,
});

const ids = (layer: { props: { data: unknown } }) =>
  (layer.props.data as { id: string }[]).map((d) => d.id);
const names = (layer: { props: { data: unknown } }) =>
  (layer.props.data as { name: string }[]).map((d) => d.name);

describe("buildFcaLayers", () => {
  it("hides disabled FCAs and keeps enabled ones on the map", () => {
    const [line] = buildFcaLayers(
      [fca({ id: "on", enabled: true }), fca({ id: "off", enabled: false })],
      null,
    );
    expect(ids(line)).toEqual(["on"]);
  });

  it("keeps a disabled FCA visible while it is selected (so it can be inspected/edited)", () => {
    const [line] = buildFcaLayers([fca({ id: "off", enabled: false })], "off");
    expect(ids(line)).toEqual(["off"]);
  });

  it("hides the labels of disabled FCAs too", () => {
    const [, , labels] = buildFcaLayers(
      [
        fca({ id: "on", enabled: true, name: "ON" }),
        fca({ id: "off", enabled: false, name: "OFF" }),
      ],
      null,
    );
    expect(names(labels)).toEqual(["ON"]);
  });
});
