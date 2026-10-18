// Status colours adapted from PasarGuard's dashboard (constants/UserSettings.ts), AGPL-3.0.
import { AlertCircleIcon, BanIcon, ClockIcon, PauseIcon, WifiIcon, type LucideIcon } from "lucide-react";
import type { ResetStrategy, User, UserStatus } from "./api";

export const statuses: UserStatus[] = ["active", "on_hold", "disabled", "limited", "expired"];
export const resetStrategies: ResetStrategy[] = ["no_reset", "day", "week", "month", "year"];

/** How each status looks: its badge and the colour of its usage bar. */
export const statusStyle: Record<UserStatus, { badge: string; bar: string; icon: LucideIcon }> = {
  active: { badge: "bg-success/12 text-success", bar: "bg-success", icon: WifiIcon },
  on_hold: { badge: "bg-info/12 text-info", bar: "bg-info", icon: PauseIcon },
  disabled: { badge: "bg-muted text-muted-foreground", bar: "bg-muted-foreground/50", icon: BanIcon },
  limited: { badge: "bg-destructive/12 text-destructive", bar: "bg-destructive", icon: AlertCircleIcon },
  expired: { badge: "bg-primary/15 text-primary", bar: "bg-primary", icon: ClockIcon },
};

export const GB = 1024 ** 3;
export const DAY = 86_400;

/** Gigabytes as typed (blank or 0 means no limit) to bytes. */
export function gbToBytes(value: string): number {
  const gb = Number(value.replace(",", "."));
  return Number.isFinite(gb) && gb > 0 ? Math.round(gb * GB) : 0;
}

/** Bytes to gigabytes for a field, without trailing noise. */
export function bytesToGb(bytes: number): string {
  if (!bytes) return "";
  return String(Math.round((bytes / GB) * 100) / 100);
}

/** Megabits per second as typed to bytes per second, and back. */
export function mbitToBytes(value: string): number {
  const mbit = Number(value.replace(",", "."));
  return Number.isFinite(mbit) && mbit > 0 ? Math.round((mbit * 1_000_000) / 8) : 0;
}
export function bytesToMbit(bytes: number): string {
  if (!bytes) return "";
  return String(Math.round(((bytes * 8) / 1_000_000) * 100) / 100);
}

/** Whole days from `now` until `at`, rounded up; negative once past. */
export function daysUntil(at: number, now = Date.now() / 1000): number {
  return Math.ceil((at - now) / DAY);
}

/** A date in the reader's calendar: Solar Hijri for Persian. */
export function formatDate(seconds: number, locale: string, time = false): string {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    ...(time ? { timeStyle: "short" } : {}),
  }).format(new Date(seconds * 1000));
}

/** "3 days" / "5 hours" / "2 minutes" ago or ahead, in the reader's language. */
export function relative(seconds: number, locale: string, now = Date.now() / 1000): string {
  const diff = seconds - now;
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const abs = Math.abs(diff);
  if (abs < 60) return format.format(Math.round(diff), "second");
  if (abs < 3600) return format.format(Math.round(diff / 60), "minute");
  if (abs < DAY) return format.format(Math.round(diff / 3600), "hour");
  if (abs < 30 * DAY) return format.format(Math.round(diff / DAY), "day");
  if (abs < 365 * DAY) return format.format(Math.round(diff / (30 * DAY)), "month");
  return format.format(Math.round(diff / (365 * DAY)), "year");
}

/** How much of its limit a user has used, 0–100 (0 without a limit). */
export function usedPercent(user: Pick<User, "data_limit" | "used_traffic">): number {
  if (!user.data_limit) return 0;
  return Math.min(100, (user.used_traffic / user.data_limit) * 100);
}
