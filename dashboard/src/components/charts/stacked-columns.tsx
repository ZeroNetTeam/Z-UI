import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { byteAxis, type Bucket } from "@/lib/series";
import { cn, formatBytes } from "@/lib/utils";

/**
 * Download and upload as stacked columns: one column per bucket, grown from
 * a single baseline, a 2px gap between the segments, a rounded top, hairline
 * grid. Hover or focus a column for its numbers; the table view shows the
 * same numbers without any pointer. Time runs left to right in every
 * language.
 */
export function StackedColumns({
  buckets,
  unit,
  table = false,
  className,
}: {
  buckets: Bucket[];
  unit: "hour" | "day";
  /** Show the numbers as a table instead. */
  table?: boolean;
  className?: string;
}) {
  const { t, i18n } = useTranslation();
  const [active, setActive] = useState<number | null>(null);
  const id = useId();
  const bytes = (n: number) => formatBytes(n, i18n.language);
  const when = (start: number) =>
    new Intl.DateTimeFormat(
      i18n.language,
      unit === "hour"
        ? { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false }
        : { month: "short", day: "numeric" },
    ).format(new Date(start));
  const axisLabel = (start: number) =>
    new Intl.DateTimeFormat(
      i18n.language,
      unit === "hour" ? { hour: "2-digit", minute: "2-digit", hour12: false } : { month: "short", day: "numeric" },
    ).format(new Date(start));

  if (table) {
    return (
      <div className={cn("max-h-72 overflow-y-auto rounded-lg border", className)}>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("monitor.time")}</TableHead>
              <TableHead>{t("monitor.download")}</TableHead>
              <TableHead>{t("monitor.upload")}</TableHead>
              <TableHead>{t("monitor.total")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {[...buckets].reverse().map((bucket) => (
              <TableRow key={bucket.start}>
                <TableCell>{when(bucket.start)}</TableCell>
                <TableCell className="tabular" dir="ltr">
                  {bytes(bucket.downlink)}
                </TableCell>
                <TableCell className="tabular" dir="ltr">
                  {bytes(bucket.uplink)}
                </TableCell>
                <TableCell className="tabular" dir="ltr">
                  {bytes(bucket.uplink + bucket.downlink)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    );
  }

  const axis = byteAxis(Math.max(0, ...buckets.map((b) => b.uplink + b.downlink)));
  const every = Math.max(1, Math.ceil(buckets.length / 6));
  const shown = active === null ? null : buckets[active];
  // A tooltip in the last columns opens to the left, so it is never cut off.
  const near = active !== null && active > buckets.length * 0.6 ? "end" : "start";

  return (
    <div className={cn("grid gap-2", className)} dir="ltr">
      <div className="flex" style={{ height: 220 }}>
        <div className="relative w-14 shrink-0 text-[11px] text-muted-foreground" aria-hidden>
          {axis.ticks.map((tick) => (
            <span key={tick} className="tabular absolute end-2 -translate-y-1/2 whitespace-nowrap" style={{ bottom: `${(tick / axis.top) * 100}%` }}>
              {tick === 0 ? "0" : bytes(tick)}
            </span>
          ))}
        </div>
        <div className="relative min-w-0 flex-1" onPointerLeave={() => setActive(null)}>
          {axis.ticks.map((tick) => (
            <div key={tick} className="absolute inset-x-0 border-t border-border/70" style={{ bottom: `${(tick / axis.top) * 100}%` }} aria-hidden />
          ))}
          <div className="absolute inset-0 flex" role="list" aria-label={t("monitor.trafficChart")}>
            {buckets.map((bucket, i) => {
              const down = (bucket.downlink / axis.top) * 100;
              const up = (bucket.uplink / axis.top) * 100;
              return (
                <div
                  key={bucket.start}
                  role="listitem"
                  tabIndex={0}
                  id={`${id}-${i}`}
                  aria-label={t("monitor.columnLabel", { when: when(bucket.start), down: bytes(bucket.downlink), up: bytes(bucket.uplink) })}
                  className={cn("relative flex min-w-0 flex-1 items-end justify-center rounded-sm outline-none", active === i && "bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/60")}
                  onPointerEnter={() => setActive(i)}
                  onFocus={() => setActive(i)}
                  onBlur={() => setActive(null)}
                >
                  <div className="flex h-full w-full max-w-6 flex-col-reverse justify-start gap-[2px] px-px" style={{ height: "100%" }}>
                    {/* A column grows from the baseline: the download first, the upload on top. */}
                    <div style={{ height: `${down}%`, background: "var(--chart-1)", minHeight: bucket.downlink > 0 ? 2 : 0 }} className={cn(bucket.uplink === 0 && "rounded-t-[4px]")} />
                    <div style={{ height: `${up}%`, background: "var(--chart-2)", minHeight: bucket.uplink > 0 ? 2 : 0 }} className="rounded-t-[4px]" />
                  </div>
                </div>
              );
            })}
          </div>
          {shown && (
            <div
              className={cn(
                "pointer-events-none absolute top-0 z-10 w-max min-w-40 rounded-lg border bg-popover px-3 py-2 text-xs shadow-lg",
                // Beside the column, on whichever side has room: never on it.
                near === "end" ? "-translate-x-[calc(100%+12px)]" : "translate-x-3",
              )}
              style={{ left: `${(((active ?? 0) + 0.5) / buckets.length) * 100}%` }}
              role="status"
            >
              <div className="mb-1 text-muted-foreground">{when(shown.start)}</div>
              {[
                { label: t("monitor.download"), value: shown.downlink, color: "var(--chart-1)" },
                { label: t("monitor.upload"), value: shown.uplink, color: "var(--chart-2)" },
              ].map((row) => (
                <div key={row.label} className="flex items-center justify-between gap-4">
                  <span className="flex items-center gap-2 text-muted-foreground">
                    <span className="inline-block h-0.5 w-3 rounded" style={{ background: row.color }} />
                    {row.label}
                  </span>
                  <span className="tabular font-semibold text-foreground">{bytes(row.value)}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
      <div className="flex ps-14 text-[11px] text-muted-foreground" aria-hidden>
        {buckets.map((bucket, i) => (
          <span key={bucket.start} className="min-w-0 flex-1 text-center whitespace-nowrap">
            {i % every === 0 ? <span className="inline-block w-0 overflow-visible">{axisLabel(bucket.start)}</span> : null}
          </span>
        ))}
      </div>
      <div className="flex items-center justify-center gap-5 text-xs text-muted-foreground">
        {[
          { label: t("monitor.download"), color: "var(--chart-1)" },
          { label: t("monitor.upload"), color: "var(--chart-2)" },
        ].map((item) => (
          <span key={item.label} className="flex items-center gap-1.5">
            <span className="inline-block size-2.5 rounded-[3px]" style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}
