import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { StackedColumns } from "@/components/charts/stacked-columns";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type UsagePoint, type User } from "@/lib/api";
import { dayBuckets } from "@/lib/series";
import { formatBytes } from "@/lib/utils";

const ranges = [7, 30, 90];

/** One user's traffic by day, over a range. */
export function UsageDialog({ user, onOpenChange }: { user: User | null; onOpenChange: (open: boolean) => void }) {
  const { t, i18n } = useTranslation();
  const [days, setDays] = useState(7);
  const [table, setTable] = useState(false);
  const usage = useQuery({
    queryKey: ["users", user?.id, "usage", days],
    queryFn: () => api<UsagePoint[]>(`users/${user!.id}/usage?days=${days}`),
    enabled: user !== null,
  });
  const buckets = useMemo(() => dayBuckets(usage.data ?? [], days), [usage.data, days]);
  const total = buckets.reduce((sum, b) => ({ up: sum.up + b.uplink, down: sum.down + b.downlink }), { up: 0, down: 0 });
  const bytes = (n: number) => formatBytes(n, i18n.language);

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
            <div className="flex items-center gap-3 text-xs text-muted-foreground">
              <span>
                {t("monitor.download")}: <span dir="ltr" className="tabular font-medium text-foreground">{bytes(total.down)}</span>
              </span>
              <span>
                {t("monitor.upload")}: <span dir="ltr" className="tabular font-medium text-foreground">{bytes(total.up)}</span>
              </span>
              <Button size="sm" variant="ghost" aria-pressed={table} onClick={() => setTable(!table)}>
                {table ? t("monitor.showChart") : t("monitor.showTable")}
              </Button>
            </div>
          </div>
          {usage.isPending ? <Skeleton className="h-64" /> : <StackedColumns buckets={buckets} unit="day" table={table} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}
