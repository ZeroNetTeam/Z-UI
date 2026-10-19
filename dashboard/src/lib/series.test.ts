import { describe, expect, it } from "vitest";
import type { UsagePoint } from "./api";
import { byteAxis, dayBuckets, hourBuckets, niceCeil } from "./series";

const point = (iso: string, uplink: number, downlink: number): UsagePoint => ({
  hour: Math.floor(new Date(iso).getTime() / 1000),
  uplink,
  downlink,
});

describe("usage buckets", () => {
  it("lays the last hours out oldest first, with empty hours as zero", () => {
    const now = new Date("2026-10-19T12:30:00Z").getTime();
    const buckets = hourBuckets(
      [point("2026-10-19T12:00:00Z", 1, 2), point("2026-10-19T10:00:00Z", 10, 20), point("2026-10-18T00:00:00Z", 99, 99)],
      3,
      now,
    );
    expect(buckets.map((b) => [b.uplink, b.downlink])).toEqual([[10, 20], [0, 0], [1, 2]]);
    expect(buckets[2].start - buckets[1].start).toBe(3_600_000);
  });

  it("adds the hours of a local day together, and drops days outside the range", () => {
    const now = new Date(2026, 9, 19, 15, 0);
    const buckets = dayBuckets(
      [
        { hour: new Date(2026, 9, 19, 1).getTime() / 1000, uplink: 1, downlink: 2 },
        { hour: new Date(2026, 9, 19, 14).getTime() / 1000, uplink: 3, downlink: 4 },
        { hour: new Date(2026, 9, 17, 9).getTime() / 1000, uplink: 5, downlink: 6 },
        { hour: new Date(2026, 8, 1, 9).getTime() / 1000, uplink: 99, downlink: 99 },
      ],
      3,
      now,
    );
    expect(buckets.map((b) => [b.uplink, b.downlink])).toEqual([[5, 6], [0, 0], [4, 6]]);
  });
});

describe("axes", () => {
  it("rounds up to 1, 2 or 5 of a power of ten", () => {
    expect([0.7, 1, 1.1, 2.4, 3.4, 5.1, 70].map(niceCeil)).toEqual([1, 1, 2, 5, 5, 10, 100]);
    expect(niceCeil(0)).toBe(1);
  });

  it("tops a byte axis at a clean number of its own unit", () => {
    const gb = 1024 ** 3;
    expect(byteAxis(3.4 * gb).top).toBe(5 * gb);
    expect(byteAxis(3.4 * gb).ticks).toEqual([0, 2.5 * gb, 5 * gb]);
    expect(byteAxis(300).top).toBe(500);
    expect(byteAxis(0).ticks).toEqual([0, 1024]);
  });
});
