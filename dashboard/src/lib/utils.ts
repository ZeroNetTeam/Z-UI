import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/** Join class names, letting later Tailwind classes win over earlier ones. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** A byte count for people: `1.4 GB`. */
export function formatBytes(bytes: number, locale: string): string {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(value)} ${units[unit]}`;
}

/** Bytes per second for people: `1.4 MB/s`. */
export function formatRate(bytesPerSecond: number, locale: string): string {
  return `${formatBytes(bytesPerSecond, locale)}/s`;
}

/** A share as a whole percent: `42%`. */
export function formatPercent(percent: number, locale: string): string {
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 0 }).format(percent / 100);
}

/** A stretch of time in its two biggest units: `3d 4h`, `12m`. */
export function formatDuration(seconds: number, locale: string): string {
  const unit = (value: number, name: "day" | "hour" | "minute" | "second") =>
    new Intl.NumberFormat(locale, { style: "unit", unit: name, unitDisplay: "narrow" }).format(value);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${unit(days, "day")} ${unit(hours, "hour")}`;
  if (hours > 0) return `${unit(hours, "hour")} ${unit(minutes, "minute")}`;
  if (minutes > 0) return unit(minutes, "minute");
  return unit(Math.max(0, Math.floor(seconds)), "second");
}
