import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  ActivityIcon,
  CpuIcon,
  HardDriveIcon,
  MemoryStickIcon,
  NetworkIcon,
  RefreshCwIcon,
  UsersIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Meter } from "@/components/charts/meter";
import { Sparkline } from "@/components/charts/sparkline";
import { StackedColumns } from "@/components/charts/stacked-columns";
import { PageHeader } from "@/components/layout/layout";
import { QuickActions } from "@/components/quick-actions";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type Health, type SystemView, type TrafficView } from "@/lib/api";
import { dayBuckets, hourBuckets } from "@/lib/series";
import { cn, formatBytes, formatDuration, formatPercent, formatRate } from "@/lib/utils";
import { useUserCounts } from "@/pages/users";

/** The ranges the traffic can be looked at over: how many columns, of what. */
const ranges = [
  { id: "24h", days: 1, unit: "hour", count: 24 },
  { id: "7d", days: 7, unit: "day", count: 7 },
  { id: "30d", days: 30, unit: "day", count: 30 },
  { id: "90d", days: 90, unit: "day", count: 90 },
] as const;

function Tile({
  icon: Icon,
  title,
  hint,
  children,
}: {
  icon: LucideIcon;
  title: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="flex-row items-center gap-3 space-y-0 pb-2">
        <span className="flex size-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Icon className="size-4" />
        </span>
        <div className="min-w-0">
          <CardTitle className="text-sm font-medium text-muted-foreground">{title}</CardTitle>
          {hint && <CardDescription className="mt-0.5 truncate">{hint}</CardDescription>}
        </div>
      </CardHeader>
      <CardContent className="grid gap-3">{children}</CardContent>
    </Card>
  );
}

const Figure = ({ children }: { children: ReactNode }) => <div className="tabular text-2xl font-semibold">{children}</div>;

/** A line under a tile, for the figure's context. */
const Fine = ({ children }: { children: ReactNode }) => <div className="text-xs text-muted-foreground">{children}</div>;

function ServerSection({ system }: { system: SystemView | undefined }) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const now = system?.now;
  const history = system?.history ?? [];
  const bytes = (n: number) => formatBytes(n, locale);
  const share = (used: number | null | undefined, total: number | null | undefined) =>
    used != null && total ? used / total : null;
  const number = (n: number, digits = 2) => new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(n);

  const memShare = share(now?.mem_used, now?.mem_total);
  const diskShare = share(now?.disk_used, now?.disk_total);
  const unavailable = <Fine>{t("monitor.unavailable")}</Fine>;

  return (
    <section aria-labelledby="server-title" className="mt-8">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 id="server-title" className="text-base font-semibold">
          {t("monitor.server")}
        </h2>
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="size-1.5 animate-pulse rounded-full bg-success" aria-hidden />
          {t("monitor.live")}
        </span>
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Tile icon={CpuIcon} title={t("monitor.cpu")} hint={now ? t("monitor.cores", { count: now.cores }) : undefined}>
          {!system ? (
            <Skeleton className="h-16" />
          ) : now?.cpu == null ? (
            <>
              <Figure>—</Figure>
              {unavailable}
            </>
          ) : (
            <>
              <Figure>{formatPercent(now.cpu, locale)}</Figure>
              <Meter value={now.cpu / 100} label={t("monitor.cpu")} />
              <Sparkline
                series={[{ values: history.map((p) => p.cpu), color: "var(--chart-1)" }]}
                max={100}
                format={(v) => formatPercent(v, locale)}
                label={t("monitor.cpuHistory")}
              />
            </>
          )}
        </Tile>

        <Tile
          icon={MemoryStickIcon}
          title={t("monitor.memory")}
          hint={now?.mem_total ? t("monitor.of", { total: bytes(now.mem_total) }) : undefined}
        >
          {!system ? (
            <Skeleton className="h-16" />
          ) : memShare === null ? (
            <>
              <Figure>—</Figure>
              {unavailable}
            </>
          ) : (
            <>
              <Figure>
                <span dir="ltr">{bytes(now!.mem_used!)}</span>
              </Figure>
              <Meter value={memShare} label={t("monitor.memory")} />
              <Sparkline
                series={[{ values: history.map((p) => p.mem), color: "var(--chart-1)" }]}
                max={now!.mem_total ?? undefined}
                format={bytes}
                label={t("monitor.memoryHistory")}
              />
              {now!.swap_total ? (
                <Fine>
                  {t("monitor.swap")}: <span dir="ltr">{bytes(now!.swap_used ?? 0)}</span> / <span dir="ltr">{bytes(now!.swap_total)}</span>
                </Fine>
              ) : null}
            </>
          )}
        </Tile>

        <Tile
          icon={HardDriveIcon}
          title={t("monitor.disk")}
          hint={now?.disk_total ? t("monitor.of", { total: bytes(now.disk_total) }) : undefined}
        >
          {!system ? (
            <Skeleton className="h-16" />
          ) : diskShare === null ? (
            <>
              <Figure>—</Figure>
              {unavailable}
            </>
          ) : (
            <>
              <Figure>
                <span dir="ltr">{bytes(now!.disk_used!)}</span>
              </Figure>
              <Meter value={diskShare} label={t("monitor.disk")} />
              <Fine>{t("monitor.diskHint", { percent: formatPercent(diskShare * 100, locale) })}</Fine>
            </>
          )}
        </Tile>

        <Tile icon={NetworkIcon} title={t("monitor.network")} hint={t("monitor.networkHint")}>
          {!system ? (
            <Skeleton className="h-16" />
          ) : now?.rx_rate == null ? (
            <>
              <Figure>—</Figure>
              {unavailable}
            </>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-2">
                {[
                  { label: t("monitor.download"), rate: now.rx_rate, color: "var(--chart-1)" },
                  { label: t("monitor.upload"), rate: now.tx_rate ?? 0, color: "var(--chart-2)" },
                ].map((item) => (
                  <div key={item.label} className="min-w-0">
                    <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span className="inline-block size-2 rounded-[2px]" style={{ background: item.color }} />
                      {item.label}
                    </div>
                    <div className="tabular truncate text-base font-semibold" dir="ltr">
                      {formatRate(item.rate, locale)}
                    </div>
                  </div>
                ))}
              </div>
              <Sparkline
                series={[
                  { values: history.map((p) => p.rx), color: "var(--chart-1)" },
                  { values: history.map((p) => p.tx), color: "var(--chart-2)" },
                ]}
                format={(v) => formatRate(v, locale)}
                label={t("monitor.networkHistory")}
              />
            </>
          )}
        </Tile>
      </div>
      {now && (
        <p className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
          {now.load && (
            <span>
              {t("monitor.load")}: <span className="tabular" dir="ltr">{now.load.map((v) => number(v)).join(" · ")}</span>
            </span>
          )}
          {now.uptime != null && (
            <span>
              {t("monitor.uptime")}: <span className="tabular">{formatDuration(now.uptime, locale)}</span>
            </span>
          )}
          {now.rx_total != null && now.tx_total != null && (
            <span>
              {t("monitor.sinceBoot")}: ↓ <span dir="ltr">{bytes(now.rx_total)}</span> · ↑ <span dir="ltr">{bytes(now.tx_total)}</span>
            </span>
          )}
          <span>
            {t("monitor.panel")}: <span className="tabular">{formatDuration(system?.panel_uptime ?? 0, locale)}</span>
            {now.panel_rss != null && (
              <>
                {" · "}
                <span dir="ltr">{bytes(now.panel_rss)}</span>
              </>
            )}
          </span>
        </p>
      )}
    </section>
  );
}

function TrafficSection() {
  const { t, i18n } = useTranslation();
  const [rangeId, setRangeId] = useState<(typeof ranges)[number]["id"]>("7d");
  const [table, setTable] = useState(false);
  const range = ranges.find((r) => r.id === rangeId)!;
  const traffic = useQuery({
    queryKey: ["monitor", "traffic", rangeId],
    queryFn: () => api<TrafficView>(`monitor/traffic?days=${range.days}&top=5`),
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  const buckets = useMemo(() => {
    const points = traffic.data?.points ?? [];
    return range.unit === "hour" ? hourBuckets(points, range.count) : dayBuckets(points, range.count);
  }, [traffic.data, range]);
  const totals = buckets.reduce((sum, b) => ({ up: sum.up + b.uplink, down: sum.down + b.downlink }), { up: 0, down: 0 });
  const bytes = (n: number) => formatBytes(n, i18n.language);
  const top = traffic.data?.top ?? [];
  const biggest = Math.max(1, ...top.map((u) => u.uplink + u.downlink));
  // While the next range loads, the last one stays on screen, faded.
  const refreshing = traffic.isFetching && traffic.isPlaceholderData;

  return (
    <section aria-labelledby="traffic-title" className="mt-8">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <h2 id="traffic-title" className="text-base font-semibold">
          {t("monitor.traffic")}
        </h2>
        <div className="flex rounded-lg border p-1 text-sm" role="radiogroup" aria-label={t("monitor.range")}>
          {ranges.map((item) => (
            <button
              key={item.id}
              type="button"
              role="radio"
              aria-checked={item.id === rangeId}
              onClick={() => setRangeId(item.id)}
              className={cn(
                "rounded-md px-3 py-1 transition-colors",
                item.id === rangeId ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent",
              )}
            >
              {t(`monitor.range_${item.id}`)}
            </button>
          ))}
        </div>
      </div>
      <div className={cn("grid gap-4 transition-opacity lg:grid-cols-3", refreshing && "opacity-60")}>
        <Card className="lg:col-span-2">
          <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
            <div>
              <CardTitle>{t("monitor.usage")}</CardTitle>
              <div className="tabular mt-1 text-2xl font-semibold" dir="ltr">
                {bytes(totals.up + totals.down)}
              </div>
              <CardDescription className="mt-1">
                {t("monitor.download")} <span dir="ltr">{bytes(totals.down)}</span> · {t("monitor.upload")} <span dir="ltr">{bytes(totals.up)}</span>
              </CardDescription>
            </div>
            <Button variant="ghost" size="sm" aria-pressed={table} onClick={() => setTable(!table)}>
              {table ? t("monitor.showChart") : t("monitor.showTable")}
            </Button>
          </CardHeader>
          <CardContent>
            {traffic.isPending ? <Skeleton className="h-64" /> : <StackedColumns buckets={buckets} unit={range.unit} table={table} />}
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>{t("monitor.topUsers")}</CardTitle>
            <CardDescription>{t("monitor.topUsersHint")}</CardDescription>
          </CardHeader>
          <CardContent>
            {traffic.isPending ? (
              <Skeleton className="h-40" />
            ) : top.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">{t("monitor.noTraffic")}</p>
            ) : (
              <ol className="grid gap-3">
                {top.map((user) => (
                  <li key={user.id} className="grid gap-1.5">
                    <div className="flex items-baseline justify-between gap-3 text-sm">
                      <span className="min-w-0 truncate font-medium" dir="ltr">
                        {user.username}
                      </span>
                      <span className="tabular shrink-0" dir="ltr">
                        {bytes(user.uplink + user.downlink)}
                      </span>
                    </div>
                    <div className="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden>
                      <div
                        className="h-full rounded-full"
                        style={{ width: `${((user.uplink + user.downlink) / biggest) * 100}%`, background: "var(--chart-1)" }}
                      />
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}

export function DashboardPage() {
  const { t, i18n } = useTranslation();
  const health = useQuery({
    queryKey: ["health"],
    queryFn: () => api<Health>("health"),
    refetchInterval: 5000,
  });
  const system = useQuery({
    queryKey: ["monitor", "system"],
    queryFn: () => api<SystemView>("monitor/system?minutes=60"),
    refetchInterval: 10_000,
  });
  const counts = useUserCounts();
  // The lifetime total comes with the traffic query; the tile reads it from there.
  const lifetime = useQuery({
    queryKey: ["monitor", "traffic", "lifetime"],
    queryFn: () => api<TrafficView>("monitor/traffic?days=1&top=0"),
    refetchInterval: 60_000,
  });
  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);

  if (health.isError) {
    return (
      <>
        <PageHeader title={t("dashboard.title")} subtitle={t("dashboard.subtitle")} />
        <Card className="border-destructive/40">
          <CardContent className="flex items-center justify-between gap-4 p-5">
            <span className="text-sm">{t("dashboard.unreachable")}</span>
            <Button variant="outline" size="sm" onClick={() => void health.refetch()}>
              <RefreshCwIcon /> {t("dashboard.retry")}
            </Button>
          </CardContent>
        </Card>
      </>
    );
  }

  const data = health.data;
  return (
    <>
      <PageHeader title={t("dashboard.title")} subtitle={t("dashboard.subtitle")} action={<QuickActions />} />
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Tile
          icon={UsersIcon}
          title={t("dashboard.users")}
          hint={counts.data ? t("dashboard.usersHint", { online: counts.data.online ?? 0, active: counts.data.active ?? 0 }) : undefined}
        >
          {counts.data ? (
            <>
              <Figure>{number(counts.data.total ?? 0)}</Figure>
              <Fine>
                {t("monitor.expiredAndLimited", {
                  expired: number(counts.data.expired ?? 0),
                  limited: number(counts.data.limited ?? 0),
                })}
              </Fine>
            </>
          ) : (
            <Skeleton className="h-12" />
          )}
        </Tile>
        <Tile icon={ActivityIcon} title={t("dashboard.sessions")} hint={t("dashboard.sessionsHint")}>
          {data ? <Figure>{number(data.core.active_sessions)}</Figure> : <Skeleton className="h-8 w-20" />}
        </Tile>
        <Tile icon={WaypointsIcon} title={t("monitor.lifetime")} hint={t("monitor.lifetimeHint")}>
          {lifetime.data ? (
            <Figure>
              <span dir="ltr">{formatBytes(lifetime.data.lifetime, i18n.language)}</span>
            </Figure>
          ) : (
            <Skeleton className="h-8 w-28" />
          )}
        </Tile>
        <Tile icon={CpuIcon} title={t("dashboard.core")} hint={data?.core.core_version}>
          {data ? (
            <>
              {data.core.running ? (
                <Badge variant="success" className="w-fit text-sm">
                  {t("dashboard.running")}
                </Badge>
              ) : (
                <Badge variant="secondary" className="w-fit text-sm">
                  {t("dashboard.stopped")}
                </Badge>
              )}
              <Fine>
                {t("monitor.inboundsCount", { count: data.core.inbounds.length })} · Z-UI {data.version}
              </Fine>
            </>
          ) : (
            <Skeleton className="h-12" />
          )}
        </Tile>
      </div>

      {data && !data.core.running && (
        <Card className="mt-4 border-primary/30 bg-primary/5">
          <CardContent className="p-5 text-sm">{t("dashboard.noCore")}</CardContent>
        </Card>
      )}

      <ServerSection system={system.data} />
      <TrafficSection />
    </>
  );
}
