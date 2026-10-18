import { useQuery } from "@tanstack/react-query";
import { ActivityIcon, CpuIcon, PackageIcon, RefreshCwIcon, UsersIcon, WaypointsIcon, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type Health } from "@/lib/api";
import { useUserCounts } from "@/pages/users";

function StatCard({
  icon: Icon,
  title,
  hint,
  children,
}: {
  icon: LucideIcon;
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="flex-row items-center gap-3 space-y-0">
        <span className="flex size-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Icon className="size-4" />
        </span>
        <div className="min-w-0">
          <CardTitle>{title}</CardTitle>
          {hint && <CardDescription className="mt-1 truncate">{hint}</CardDescription>}
        </div>
      </CardHeader>
      <CardContent className="tabular text-2xl font-semibold">{children}</CardContent>
    </Card>
  );
}

export function DashboardPage() {
  const { t, i18n } = useTranslation();
  const health = useQuery({
    queryKey: ["health"],
    queryFn: () => api<Health>("health"),
    refetchInterval: 5000,
  });
  const counts = useUserCounts();
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
  const value = (render: (data: Health) => ReactNode) =>
    data ? render(data) : <Skeleton className="h-8 w-20" />;

  return (
    <>
      <PageHeader title={t("dashboard.title")} subtitle={t("dashboard.subtitle")} />
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        <StatCard
          icon={UsersIcon}
          title={t("dashboard.users")}
          hint={counts.data ? t("dashboard.usersHint", { online: counts.data.online ?? 0, active: counts.data.active ?? 0 }) : undefined}
        >
          {counts.data ? number(counts.data.total ?? 0) : <Skeleton className="h-8 w-20" />}
        </StatCard>
        <StatCard icon={CpuIcon} title={t("dashboard.core")} hint={data?.core.core_version}>
          {value((d) =>
            d.core.running ? (
              <Badge variant="success" className="text-sm">
                {t("dashboard.running")}
              </Badge>
            ) : (
              <Badge variant="secondary" className="text-sm">
                {t("dashboard.stopped")}
              </Badge>
            ),
          )}
        </StatCard>
        <StatCard icon={WaypointsIcon} title={t("dashboard.inbounds")} hint={t("dashboard.inboundsHint")}>
          {value((d) => number(d.core.inbounds.length))}
        </StatCard>
        <StatCard icon={ActivityIcon} title={t("dashboard.sessions")} hint={t("dashboard.sessionsHint")}>
          {value((d) => number(d.core.active_sessions))}
        </StatCard>
        <StatCard icon={PackageIcon} title={t("dashboard.version")} hint={data?.product}>
          {value((d) => d.version)}
        </StatCard>
      </div>

      {data && !data.core.running && (
        <Card className="mt-4 border-primary/30 bg-primary/5">
          <CardContent className="p-5 text-sm">{t("dashboard.noCore")}</CardContent>
        </Card>
      )}

      <Card className="mt-4">
        <CardHeader>
          <CardTitle>{t("dashboard.system")}</CardTitle>
          <CardDescription>{t("dashboard.systemSoon")}</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="grid h-40 place-items-center rounded-lg border border-dashed text-muted-foreground">
            <ActivityIcon className="size-6 opacity-40" />
          </div>
        </CardContent>
      </Card>
    </>
  );
}
