import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeftRightIcon, CircleCheckIcon, Trash2Icon, UndoIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { api, post, type TakeoverView } from "@/lib/api";
import { useMe } from "@/lib/permissions";
import { windowLeft } from "@/lib/takeover";
import { formatDate } from "@/lib/users";

/** The takeover, for a sudo admin; nothing for anyone else or when there was none. */
export function useTakeover(refetchMs?: number) {
  const me = useMe();
  return useQuery({
    queryKey: ["takeover"],
    queryFn: () => api<TakeoverView | null>("takeover"),
    enabled: !!me?.is_sudo,
    refetchInterval: refetchMs,
  });
}

/** What is left of the week, in words, and the day it ends. */
function timeLeft(view: TakeoverView, t: TFunction, locale: string) {
  const left = windowLeft(view.expires_at);
  const number = (n: number) => new Intl.NumberFormat(locale).format(n);
  const text =
    left.days > 0
      ? t("takeover.daysLeft", { count: left.days, formatted: number(left.days) })
      : t("takeover.hoursLeft", { count: left.hours, formatted: number(left.hours) });
  const date = view.expires_at ? formatDate(view.expires_at, locale, true) : "";
  return { text, date };
}

/** The line on the dashboard: the old panel was replaced, and there is time to go back. */
export function TakeoverBanner() {
  const { t, i18n } = useTranslation();
  const takeover = useTakeover();
  const view = takeover.data;
  if (!view || view.phase !== "switched") return null;
  const { text } = timeLeft(view, t, i18n.language);
  return (
    <Card className="mb-4 border-primary/40">
      <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
        <div className="flex min-w-0 items-start gap-3">
          <ArrowLeftRightIcon className="mt-0.5 size-4 shrink-0 text-primary" />
          <p className="text-sm" dir="auto">
            {t("takeover.bannerBody", {
              panel: t(`migrate.kind.${view.kind}`),
              when: formatDate(view.switched_at ?? view.prepared_at, i18n.language, true),
              left: text,
            })}
          </p>
        </div>
        <Button variant="outline" size="sm" asChild>
          <Link to="/migrate">{t("takeover.open")}</Link>
        </Button>
      </CardContent>
    </Card>
  );
}

/** What can be done about a panel that was replaced: go back, or let go. */
export function TakeoverCard() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const takeover = useTakeover();
  const [asking, setAsking] = useState<"back" | "delete" | null>(null);
  const [started, setStarted] = useState(false);
  const back = useMutation({
    mutationFn: () => post<{ started: boolean }>("takeover/rollback"),
    onSuccess: () => {
      setAsking(null);
      setStarted(true);
    },
  });
  const remove = useMutation({
    mutationFn: () => post<TakeoverView>("takeover/delete-old"),
    onSuccess: (view) => {
      setAsking(null);
      client.setQueryData(["takeover"], view);
    },
  });
  const view = takeover.data;
  if (!view) return null;
  const panel = t(`migrate.kind.${view.kind}`);
  const when = (at: number | null) => (at ? formatDate(at, i18n.language, true) : "");
  const { text, date } = timeLeft(view, t, i18n.language);

  if (view.phase === "rolled_back" || view.phase === "finished" || view.phase === "prepared") {
    const key = view.phase === "prepared" ? "prepared" : view.phase === "finished" ? "deleted" : "rolledBack";
    return (
      <Card>
        <CardContent className="flex items-start gap-3 p-4 text-sm">
          <CircleCheckIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <p dir="auto">{t(`takeover.${key}`, { panel, when: when(view.ended_at ?? view.prepared_at) })}</p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="border-primary/40">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ArrowLeftRightIcon className="size-4 text-primary" /> {t("takeover.title", { panel })}
        </CardTitle>
        <CardDescription>{t("takeover.replacedOn", { when: when(view.switched_at) })}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        <p className="text-sm" dir="auto">
          {t("takeover.body", { users: view.users, inbounds: view.inbounds, date, left: text })}
        </p>
        {started && (
          <p className="rounded-md border border-warning/40 p-3 text-sm" dir="auto">
            {t("takeover.started", { panel })}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button variant="destructive" disabled={!view.can_rollback || started} onClick={() => setAsking("back")}>
            <UndoIcon /> {t("takeover.back")}
          </Button>
          <Button variant="outline" disabled={!view.can_delete || started} onClick={() => setAsking("delete")}>
            <Trash2Icon /> {t("takeover.delete")}
          </Button>
        </div>
        {!view.can_rollback && <p className="text-xs text-warning">{t("takeover.cannotBack")}</p>}
      </CardContent>

      <Dialog open={asking === "back"} onOpenChange={(open) => !open && setAsking(null)}>
        <DialogContent title={t("takeover.backTitle", { panel })} className="max-w-md">
          <div className="grid gap-4">
            <ul className="grid list-disc gap-1.5 ps-5 text-sm text-muted-foreground">
              <li>{t("takeover.back1", { panel })}</li>
              <li>{t("takeover.back2", { panel })}</li>
              <li>{t("takeover.back3", { panel })}</li>
              <li>{t("takeover.back4", { panel })}</li>
            </ul>
            <FormError error={back.error} />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setAsking(null)}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" disabled={back.isPending} onClick={() => back.mutate()}>
                {back.isPending ? t("takeover.starting") : t("takeover.backNow")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={asking === "delete"} onOpenChange={(open) => !open && setAsking(null)}>
        <DialogContent title={t("takeover.deleteTitle", { panel })} className="max-w-md">
          <div className="grid gap-4">
            <p className="text-sm text-muted-foreground" dir="auto">
              {t("takeover.deleteBody", { panel, date })}
            </p>
            <FormError error={remove.error} />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setAsking(null)}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" disabled={remove.isPending} onClick={() => remove.mutate()}>
                {t("takeover.deleteNow")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
