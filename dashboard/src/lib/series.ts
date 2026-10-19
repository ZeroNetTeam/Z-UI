/**
 * Turning usage points into what a chart draws: columns of an hour or a
 * local day, and axis ticks that are clean numbers.
 */
import type { UsagePoint } from "./api";

export interface Bucket {
  /** Start of the hour or day, in milliseconds. */
  start: number;
  uplink: number;
  downlink: number;
}

const HOUR = 3_600_000;

/** The last `count` hours, each a column, oldest first; hours with no traffic are zero. */
export function hourBuckets(points: UsagePoint[], count: number, now = Date.now()): Bucket[] {
  const last = Math.floor(now / HOUR) * HOUR;
  const buckets: Bucket[] = [];
  for (let i = count - 1; i >= 0; i -= 1) buckets.push({ start: last - i * HOUR, uplink: 0, downlink: 0 });
  for (const point of points) {
    const slot = Math.round((point.hour * 1000 - buckets[0].start) / HOUR);
    if (slot < 0 || slot >= count) continue;
    buckets[slot].uplink += point.uplink;
    buckets[slot].downlink += point.downlink;
  }
  return buckets;
}

/** The last `count` local days, each a column, oldest first; days with no traffic are zero. */
export function dayBuckets(points: UsagePoint[], count: number, now = new Date()): Bucket[] {
  const key = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
  const buckets: Bucket[] = [];
  const index = new Map<string, number>();
  for (let i = count - 1; i >= 0; i -= 1) {
    const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i);
    index.set(key(day), buckets.length);
    buckets.push({ start: day.getTime(), uplink: 0, downlink: 0 });
  }
  for (const point of points) {
    const slot = index.get(key(new Date(point.hour * 1000)));
    if (slot === undefined) continue;
    buckets[slot].uplink += point.uplink;
    buckets[slot].downlink += point.downlink;
  }
  return buckets;
}

/** The smallest of 1, 2, 5 × a power of ten that holds `value`. */
export function niceCeil(value: number): number {
  if (!(value > 0)) return 1;
  const exponent = Math.floor(Math.log10(value));
  const fraction = value / 10 ** exponent;
  const nice = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  return nice * 10 ** exponent;
}

/**
 * A top for a byte axis and the ticks up to it (bottom first). The top is a
 * clean number in the unit it will be shown in: 3.4 GB gets 5 GB, not
 * 5.37 billion bytes.
 */
export function byteAxis(max: number, ticks = 3): { top: number; ticks: number[] } {
  if (!(max > 0)) return { top: 1024, ticks: [0, 1024] };
  const power = Math.min(5, Math.max(0, Math.floor(Math.log(max) / Math.log(1024))));
  const unit = 1024 ** power;
  const top = niceCeil(max / unit) * unit;
  return { top, ticks: Array.from({ length: ticks }, (_, i) => (top * i) / (ticks - 1)) };
}
