import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, formatPercent, formatRate } from "./utils";

describe("formatBytes", () => {
  it("picks the largest whole unit", () => {
    expect(formatBytes(0, "en")).toBe("0 B");
    expect(formatBytes(1023, "en")).toBe("1,023 B");
    expect(formatBytes(1536, "en")).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 ** 3, "en")).toBe("5 GB");
    expect(formatBytes(150 * 1024 ** 4, "en")).toBe("150 TB");
  });

  it("writes Persian digits for Persian", () => {
    expect(formatBytes(1536, "fa")).toBe("۱٫۵ KB");
  });
});

describe("formatters for the dashboard", () => {
  it("writes rates, shares and durations", () => {
    expect(formatRate(1536, "en")).toBe("1.5 KB/s");
    expect(formatPercent(42.4, "en")).toBe("42%");
    expect(formatDuration(3 * 86_400 + 4 * 3600 + 59, "en")).toBe("3d 4h");
    expect(formatDuration(2 * 3600 + 5 * 60, "en")).toBe("2h 5m");
    expect(formatDuration(12 * 60 + 30, "en")).toBe("12m");
    expect(formatDuration(9, "en")).toBe("9s");
    expect(formatDuration(3600, "fa")).toMatch(/^[۰-۹]/);
  });
});
