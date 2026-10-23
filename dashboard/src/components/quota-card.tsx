import { TriangleAlertIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { quotaPercent } from "@/lib/admin-form";
import { useMe } from "@/lib/permissions";
import { cn, formatBytes, isolate } from "@/lib/utils";

function Meter({ label, text, percent }: { label: string; text: string; percent: number | null }) {
  return (
    <div className="grid min-w-48 flex-1 gap-1.5">
      <div className="flex items-baseline justify-between gap-2 text-sm">
        <span className="text-muted-foreground">{label}</span>
        <span className="tabular font-medium">{text}</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-muted" role="progressbar" aria-label={label} aria-valuenow={percent ?? undefined} aria-valuemin={0} aria-valuemax={100}>
        <div className={cn("h-full rounded-full", (percent ?? 0) >= 90 ? "bg-warning" : "bg-primary")} style={{ width: `${percent ?? 0}%` }} />
      </div>
    </div>
  );
}

/** What a reseller has handed out against the quotas they were given. */
export function QuotaCard() {
  const { t, i18n } = useTranslation();
  const me = useMe();
  if (!me || me.is_sudo) return null;
  const q = me.quota;
  if (q.user_limit === 0 && q.data_quota === 0) return null;
  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);
  return (
    <Card className="mb-4">
      <CardContent className="grid gap-3 p-4">
        <div className="text-sm font-medium">{t("access.quotaTitle")}</div>
        <div className="flex flex-wrap gap-x-8 gap-y-3">
          {q.user_limit > 0 && (
            <Meter
              label={t("access.quotaUsers")}
              text={t("access.quotaOf", { used: isolate(number(q.users)), limit: isolate(number(q.user_limit)) })}
              percent={quotaPercent(q.users, q.user_limit)}
            />
          )}
          {q.data_quota > 0 && (
            <Meter
              label={t("access.quotaData")}
              text={t("access.quotaOf", { used: isolate(formatBytes(q.allocated, i18n.language)), limit: isolate(formatBytes(q.data_quota, i18n.language)) })}
              percent={quotaPercent(q.allocated, q.data_quota)}
            />
          )}
        </div>
        {q.data_quota > 0 && q.unlimited > 0 && (
          <p className="flex items-center gap-1.5 text-xs text-warning">
            <TriangleAlertIcon className="size-3.5" /> {t("access.quotaUnlimitedWarn")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
