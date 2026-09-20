// @vitest-environment jsdom
import {describe, expect, it, vi} from "vitest";

import {dismissAllAlerts, onDismissAllAlerts} from "./alerts";

describe("the dismiss-all seam", () => {
  it("tells every mounted alert surface to clear", () => {
    const a = vi.fn();
    const b = vi.fn();
    const offA = onDismissAllAlerts(a);
    const offB = onDismissAllAlerts(b);

    dismissAllAlerts();

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    offA();
    offB();
  });

  it("stops telling one that has unsubscribed", () => {
    // Otherwise an unmounted component keeps being called and React warns about setting state on it.
    const listener = vi.fn();
    const off = onDismissAllAlerts(listener);
    off();

    dismissAllAlerts();

    expect(listener).not.toHaveBeenCalled();
  });

  it("is harmless when nothing is listening", () => {
    expect(() => dismissAllAlerts()).not.toThrow();
  });
});
