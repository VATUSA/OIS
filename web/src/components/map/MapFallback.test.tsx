import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {MapFallback} from "./MapFallback";

const html = (node: React.ReactElement) => renderToStaticMarkup(node);

describe("MapFallback", () => {
  it("offers a retry when the caller can re-check WebGL", () => {
    expect(html(<MapFallback onRetry={() => {}} />)).toContain("Try again");
  });

  it("omits the retry when there is nothing to retry", () => {
    expect(html(<MapFallback />)).not.toContain("Try again");
  });

  // The copy used to blame iOS Lockdown Mode outright, which is wrong on a desktop browser whose
  // GPU stack died — the case that prompted #331.
  it("does not pin the cause on iOS Lockdown Mode alone", () => {
    const markup = html(<MapFallback />);
    expect(markup).toContain("another browser");
    expect(markup).toContain("Lockdown Mode");
  });
});
