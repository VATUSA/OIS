// @vitest-environment jsdom
import {describe, expect, it} from "vitest";

import {fcaNotifiersWanted, flattenPermissions} from "./desktop-notifiers";

/**
 * The flattener decides what counts as a *new* grant, so getting it wrong either misses grants or
 * announces revokes as grants. It walks the same tree shape `hasPermission` does.
 */
describe("flattenPermissions", () => {
  it("turns the nested tree into dotted permission names", () => {
    expect(
      flattenPermissions({flow: {fca: ["read", "update"]}, tmu: {program: ["read"]}}).sort(),
    ).toEqual(["flow.fca.read", "flow.fca.update", "tmu.program.read"]);
  });

  it("handles a tree deeper than two levels", () => {
    expect(flattenPermissions({a: {b: {c: ["do"]}}})).toEqual(["a.b.c.do"]);
  });

  it("is empty for a user with nothing", () => {
    expect(flattenPermissions({})).toEqual([]);
    expect(flattenPermissions(undefined)).toEqual([]);
    expect(flattenPermissions(null)).toEqual([]);
  });

  it("ignores leaves that aren't action arrays", () => {
    // A malformed tree must not produce junk names that look like grants.
    expect(flattenPermissions({flow: {fca: "nonsense"}})).toEqual([]);
  });

  it("produces a stable set, so only real additions look new", () => {
    const before = flattenPermissions({flow: {fca: ["read"]}});
    const after = flattenPermissions({flow: {fca: ["read", "update"]}});
    const added = after.filter((p) => !before.includes(p));
    expect(added).toEqual(["flow.fca.update"]);

    // And a revoke yields no additions — it must never read as a grant.
    const revoked = flattenPermissions({flow: {fca: []}});
    expect(revoked.filter((p) => !before.includes(p))).toEqual([]);
  });
});

/**
 * The FCA detectors feed both banners and sounds, so the decision to mount them has to consider
 * both. Reading only the notification settings shipped a release where `sounds.releases` and
 * `sounds.metering` were switches that did nothing: with the banner off nothing mounted, the
 * traffic query never opened, and no sound could ever play.
 */
describe("fcaNotifiersWanted", () => {
  const off = {
    releasesOn: false,
    meteringOn: false,
    releaseSoundOn: false,
    meteringSoundOn: false,
  };

  it("mounts nothing when every switch is off", () => {
    expect(fcaNotifiersWanted(off)).toBe(false);
  });

  it("mounts for a banner alone", () => {
    expect(fcaNotifiersWanted({...off, releasesOn: true})).toBe(true);
    expect(fcaNotifiersWanted({...off, meteringOn: true})).toBe(true);
  });

  it("mounts for a sound alone — the case that was broken", () => {
    // `sounds.releases` on, `notifications.releases` off: the documented "noise without a banner".
    expect(fcaNotifiersWanted({...off, releaseSoundOn: true})).toBe(true);
    expect(fcaNotifiersWanted({...off, meteringSoundOn: true})).toBe(true);
  });

  it("mounts when one category wants a banner and the other only a sound", () => {
    expect(fcaNotifiersWanted({...off, releasesOn: true, meteringSoundOn: true})).toBe(true);
  });
});
