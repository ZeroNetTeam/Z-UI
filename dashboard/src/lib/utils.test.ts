import { describe, expect, it } from "vitest";
import { formatBytes } from "./utils";

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
