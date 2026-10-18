// Adapted from PasarGuard's dashboard (features/users/components/status-badge.tsx,
// online-badge.tsx and components/common/usage-slider-compact.tsx), AGPL-3.0.
import { useTranslation } from "react-i18next";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { User } from "@/lib/api";
import { daysUntil, formatDate, relative, statusStyle, usedPercent } from "@/lib/users";
import { cn, formatBytes } from "@/lib/utils";

/** The user's status, and when it ends or ended. */
export function StatusBadge({ user, withExpiry = true }: { user: User; withExpiry?: boolean }) {
  const { t, i18n } = useTranslation();
  const style = statusStyle[user.status];
  const Icon = style.icon;
  let expiry: string | null = null;
  if (user.status === "on_hold" && user.on_hold_duration) {
    expiry = t("users.holdFor", { count: Math.round(user.on_hold_duration / 86_400) });
  } else if (user.expire_at) {
    expiry =
      daysUntil(user.expire_at) > 0
        ? t("users.expiresIn", { when: relative(user.expire_at, i18n.language) })
        : t("users.expiredAgo", { when: relative(user.expire_at, i18n.language) });
  }
  return (
    <div className="flex flex-col items-start gap-1">
      <span className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap", style.badge)}>
        <Icon className="size-3" />
        {t(`users.status.${user.status}`)}
      </span>
      {withExpiry && expiry && (
        <span
          className="text-[11px] text-muted-foreground"
          title={user.expire_at ? formatDate(user.expire_at, i18n.language, true) : undefined}
        >
          {expiry}
        </span>
      )}
    </div>
  );
}

/** Green while connected, grey once seen, hollow when never. */
export function OnlineDot({ user }: { user: User }) {
  const { t, i18n } = useTranslation();
  const text = user.online
    ? t("users.online")
    : user.online_at
      ? t("users.lastSeen", { when: relative(user.online_at, i18n.language) })
      : t("users.neverConnected");
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          role="img"
          aria-label={text}
          className={cn(
            "inline-block size-2.5 shrink-0 rounded-full",
            user.online ? "bg-success shadow-[0_0_0_3px] shadow-success/20" : user.online_at ? "bg-muted-foreground/40" : "border border-muted-foreground/50",
          )}
        />
      </TooltipTrigger>
      <TooltipContent>{text}</TooltipContent>
    </Tooltip>
  );
}

/** Used against the limit, with the lifetime total beside it. */
export function UsageBar({ user }: { user: User }) {
  const { t, i18n } = useTranslation();
  const bytes = (n: number) => formatBytes(n, i18n.language);
  const unlimited = !user.data_limit;
  return (
    <div className="grid min-w-24 gap-1.5 text-xs text-muted-foreground md:min-w-36">
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className={cn("h-full rounded-full transition-[width]", statusStyle[user.status].bar, unlimited && "opacity-40")}
          style={{ width: `${unlimited ? 100 : usedPercent(user)}%` }}
        />
      </div>
      <div className="flex items-center justify-between gap-2">
        <span dir="ltr" className="tabular">
          {bytes(user.used_traffic)} / {unlimited ? "∞" : bytes(user.data_limit)}
        </span>
        <span className="hidden lg:inline">
          {t("users.lifetime")}: <span dir="ltr" className="tabular">{bytes(user.lifetime_traffic)}</span>
        </span>
      </div>
    </div>
  );
}
