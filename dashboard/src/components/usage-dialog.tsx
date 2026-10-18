import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api, type UsagePoint, type User } from "@/lib/api";
import { formatBytes } from "@/lib/utils";

/** Hourly points summed by local day, every day of the range present. */
export function byDay(points: UsagePoint[], days: number, now = new Date()) {
  const key = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
  const out: { day: Date; uplink: number; downlink: number }[] = [];
  const index = new Map<string, number>();
  for (let i = days - 1; i >= 0; i -= 1) {
    const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i);
    index.set(key(day), out.length);
    out.push({ day, uplink: 0, downlink: 0 });
  }
  for (const point of points) {
    const at = index.get(key(new Date(point.hour * 1000)));
    if (at === undefined) continue;
    out[at].uplink += point.uplink;
    out[at].downlink += point.downlink;
  }
  return out;
}

const ranges = [7, 30, 90];

export function UsageDialog({ user, onOpenChange }: { user: User | null; onOpenChange: (open: boolean) => void }) {
  const { t, i18n } = useTranslation();
  const [days, setDays] = useState(7);
  const usage = useQuery({
    queryKey: ["users", user?.id, "usage", days],
    queryFn: () => api<UsagePoint[]>(`users/${user!.id}/usage?days=${days}`),
    enabled: user !== null,
  });
  const bars = useMemo(() => byDay(usage.data ?? [], days), [usage.data, days]);
  const top = Math.max(1, ...bars.map((bar) => bar.uplink + bar.downlink));
  const total = bars.reduce((sum, bar) => ({ up: sum.up + bar.uplink, down: sum.down + bar.downlink }), { up: 0, down: 0 });
  const bytes = (n: number) => formatBytes(n, i18n.language);
  const day = (date: Date) => new Intl.DateTimeFormat(i18n.language, { month: "short", day: "numeric" }).format(date);

  return (
    <Dialog open={user !== null} onOpenChange={onOpenChange}>
      <DialogContent title={t("users.usageTitle", { name: user?.username })} className="max-w-2xl">
        <div className="grid gap-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex gap-1">
              {ranges.map((range) => (
                <Button key={range} size="sm" variant={range === days ? "secondary" : "ghost"} onClick={() => setDays(range)}>
                  {t("common.days", { count: range })}
                </Button>
              ))}
            </div>
            <div className="flex gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5">
                <span className="size-2 rounded-full bg-primary" /> {t("users.download")}: <span dir="ltr">{bytes(total.down)}</span>
              </span>
              <span className="flex items-center gap-1.5">
                <span className="size-2 rounded-full bg-info" /> {t("users.upload")}: <span dir="ltr">{bytes(total.up)}</span>
              </span>
            </div>
          </div>
          {usage.isPending ? (
            <Skeleton className="h-48" />
          ) : (
            <div className="flex h-48 items-end gap-[3px] rounded-lg border p-3" dir="ltr">
              {bars.map((bar) => (
                <Tooltip key={bar.day.getTime()}>
                  <TooltipTrigger asChild>
                    <div className="flex h-full min-w-0 flex-1 flex-col justify-end rounded-sm hover:bg-accent/60">
                      <div className="rounded-t-sm bg-info/80" style={{ height: `${(bar.uplink / top) * 100}%` }} />
                      <div className="bg-primary" style={{ height: `${(bar.downlink / top) * 100}%` }} />
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <div className="font-medium">{day(bar.day)}</div>
                    <div dir="ltr">↓ {bytes(bar.downlink)} · ↑ {bytes(bar.uplink)}</div>
                  </TooltipContent>
                </Tooltip>
              ))}
            </div>
          )}
          <div className="flex justify-between text-[11px] text-muted-foreground" dir="ltr">
            <span>{bars[0] && day(bars[0].day)}</span>
            <span>{bars.at(-1) && day(bars.at(-1)!.day)}</span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
