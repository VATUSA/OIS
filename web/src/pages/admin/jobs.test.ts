import {describe, expect, it} from "vitest";

import {formatBytes} from "./jobs";

describe("formatBytes", () => {
  it("keeps small values in bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("scales up through KB/MB/GB/TB, one decimal place under 10", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
    expect(formatBytes(1024 ** 4)).toBe("1.0 TB");
  });

  it("drops the decimal once the value is 10 or more in its unit", () => {
    expect(formatBytes(12 * 1024)).toBe("12 KB");
    expect(formatBytes(1024 ** 4 * 12)).toBe("12 TB");
  });

  it("never scales past TB", () => {
    expect(formatBytes(1024 ** 5)).toBe("1024 TB");
  });
});
