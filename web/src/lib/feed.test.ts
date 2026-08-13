import {describe, expect, it} from "vitest";

import {feedIsStale} from "./feed";

const NOW = 1_700_000_000_000;
const iso = (msAgo: number) => new Date(NOW - msAgo).toISOString();
const STALE = 90_000;

describe("feedIsStale", () => {
  it("is not stale for a healthy, recent feed", () => {
    expect(feedIsStale({ healthy: true, last_updated: iso(20_000) }, NOW, STALE)).toBe(
      false,
    );
  });

  it("is stale when the last fetch errored, even if recent", () => {
    expect(feedIsStale({ healthy: false, last_updated: iso(5_000) }, NOW, STALE)).toBe(
      true,
    );
  });

  it("is stale when last_updated is older than the threshold (hung poller)", () => {
    expect(
      feedIsStale({ healthy: true, last_updated: iso(120_000) }, NOW, STALE),
    ).toBe(true);
  });

  it("is stale when there is no last_updated at all", () => {
    expect(feedIsStale({ healthy: true, last_updated: null }, NOW, STALE)).toBe(true);
  });

  it("treats no status as not stale (nothing to warn about yet)", () => {
    expect(feedIsStale(undefined, NOW, STALE)).toBe(false);
  });
});
