import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BotIcon, CircleCheckIcon, CircleXIcon, SendIcon, UndoIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import { api, post, put, type ChannelStatus, type NotifyChannel, type NotifyView } from "@/lib/api";
import { fromView, isDirty, parseChatIds, toPayload, type NotifyForm } from "@/lib/notify-form";
import { relative } from "@/lib/users";

/** How the last delivery through a channel went. */
function LastResult({ status }: { status: ChannelStatus }) {
  const { t, i18n } = useTranslation();
  if (!status.ok_at && !status.failed_at) return <p className="text-xs text-muted-foreground">{t("notify.neverSent")}</p>;
  const failed = status.failed_at !== null && (status.ok_at === null || status.failed_at >= status.ok_at);
  return failed ? (
    <p className="flex items-start gap-1.5 text-xs text-destructive" dir="auto">
      <CircleXIcon className="mt-0.5 size-3.5 shrink-0" />
      <span>
        {t("notify.lastFailed", { when: relative(status.failed_at ?? 0, i18n.language) })}: {status.error}
      </span>
    </p>
  ) : (
    <p className="flex items-center gap-1.5 text-xs text-success">
      <CircleCheckIcon className="size-3.5 shrink-0" /> {t("notify.lastOk", { when: relative(status.ok_at ?? 0, i18n.language) })}
    </p>
  );
}

/** Send a test through a channel (of what is saved) and show the real answer. */
function TestButton({ channel, disabled }: { channel: NotifyChannel; disabled: boolean }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const test = useMutation({
    mutationFn: () => post<{ delivered: number; bot: string | null }>("notifications/test", { channel }),
    onSettled: () => void client.invalidateQueries({ queryKey: ["notifications"] }),
  });
  return (
    <div className="grid gap-2">
      <Button type="button" variant="outline" size="sm" className="justify-self-start" disabled={disabled || test.isPending} onClick={() => test.mutate()}>
        <SendIcon /> {test.isPending ? t("notify.testing") : t("notify.test")}
      </Button>
      {disabled && <p className="text-xs text-muted-foreground">{t("notify.saveFirst")}</p>}
      <FormError error={test.error} />
      {test.isSuccess && (
        <p className="flex items-center gap-1.5 text-sm text-success">
          <CircleCheckIcon className="size-4" />
          {channel === "telegram" ? t("notify.testedTelegram", { count: test.data.delivered }) : t("notify.tested")}
        </p>
      )}
    </div>
  );
}

const eventRows: { key: "data_low" | "expiring" | "limited" | "expired" | "core" | "login_failed" | "login_new" | "backup" | "certificate" }[] = [
  { key: "data_low" },
  { key: "expiring" },
  { key: "limited" },
  { key: "expired" },
  { key: "core" },
  { key: "login_failed" },
  { key: "login_new" },
  { key: "backup" },
  { key: "certificate" },
];

export function NotificationsPage() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const stored = useQuery({ queryKey: ["notifications"], queryFn: () => api<NotifyView>("notifications"), refetchInterval: 10_000 });
  const [form, setForm] = useState<NotifyForm | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const view = stored.data?.settings;

  // The form starts from what is stored, and is not trampled by a refresh while it is being edited.
  useEffect(() => {
    if (view && (form === null || !isDirty(form, view))) setForm(fromView(view));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view]);

  const save = useMutation({
    mutationFn: () => {
      if (!form || !view) throw new Error("not ready");
      return put<NotifyView>("notifications", toPayload(form, view));
    },
    onSuccess: (saved) => {
      client.setQueryData(["notifications"], saved);
      setForm(fromView(saved.settings));
    },
  });

  if (stored.isPending || !stored.data || !form || !view) {
    return (
      <>
        <PageHeader title={t("notify.title")} subtitle={t("notify.subtitle")} />
        <FormError error={stored.error} />
        <Skeleton className="h-64" />
      </>
    );
  }
  const set = (patch: Partial<NotifyForm>) => setForm((current) => (current ? { ...current, ...patch } : current));
  const setEvent = (patch: Partial<NotifyForm["events"]>) => set({ events: { ...form.events, ...patch } });
  const dirty = isDirty(form, view);
  const bad = parseChatIds(form.chatIds).bad;
  const status = stored.data.status;
  const bot = stored.data.bot;
  const submit = () => {
    if (bad.length > 0) return setProblem(t("notify.badChats", { words: bad.join(", ") }));
    setProblem(null);
    save.mutate();
  };

  return (
    <>
      <PageHeader title={t("notify.title")} subtitle={t("notify.subtitle")} />
      <div className="grid gap-4">
        <Card>
          <CardHeader>
            <CardTitle>{t("notify.general")}</CardTitle>
            <CardDescription>{t("notify.generalBody")}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <span className="text-sm font-medium">{t("notify.language")}</span>
              <Segmented
                label={t("notify.language")}
                value={form.language}
                onChange={(language) => set({ language })}
                options={[
                  { value: "fa", label: "فارسی" },
                  { value: "en", label: "English" },
                ]}
              />
            </div>
            <Field
              label={t("notify.proxy")}
              value={form.proxy}
              onChange={(e) => set({ proxy: e.target.value })}
              dir="ltr"
              placeholder="socks5://127.0.0.1:10808"
              hint={t("notify.proxyHint")}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
            <div>
              <CardTitle className="flex items-center gap-2">
                <BotIcon className="size-4 text-primary" /> {t("notify.telegram")}
              </CardTitle>
              <CardDescription className="mt-1 max-w-2xl">{t("notify.telegramBody")}</CardDescription>
            </div>
            {view.telegram.enabled && (
              <Badge variant={bot.running ? "success" : "secondary"} dir="ltr">
                {bot.username ? `@${bot.username}` : t("notify.bot")} · {bot.running ? t("notify.listening") : t("notify.notListening")}
              </Badge>
            )}
          </CardHeader>
          <CardContent className="grid gap-4">
            <SwitchField label={t("notify.telegramOn")} checked={form.telegramEnabled} onCheckedChange={(telegramEnabled) => set({ telegramEnabled })} />
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("notify.token")}
                type="password"
                value={form.token}
                onChange={(e) => set({ token: e.target.value })}
                dir="ltr"
                autoComplete="off"
                placeholder={form.tokenSet ? t("notify.secretSet") : "123456:ABC-DEF…"}
                hint={t("notify.tokenHint")}
              />
              <Field
                label={t("notify.apiBase")}
                value={form.apiBase}
                onChange={(e) => set({ apiBase: e.target.value })}
                dir="ltr"
                hint={t("notify.apiBaseHint")}
              />
            </div>
            <TextareaField
              label={t("notify.chats")}
              value={form.chatIds}
              onChange={(e) => set({ chatIds: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-20 font-mono text-xs"
              placeholder={"-1001234567890\n123456789"}
              hint={t("notify.chatsHint")}
            />
            <div className="grid gap-3 sm:grid-cols-2">
              <SwitchField label={t("notify.userBot")} hint={t("notify.userBotHint")} checked={form.userBot} onCheckedChange={(userBot) => set({ userBot })} />
              <SwitchField label={t("notify.notifyUsers")} hint={t("notify.notifyUsersHint")} checked={form.notifyUsers} onCheckedChange={(notifyUsers) => set({ notifyUsers })} />
            </div>
            {bot.error && view.telegram.enabled && (
              <p className="text-xs text-destructive" dir="auto">
                {bot.error}
              </p>
            )}
            <LastResult status={status.telegram} />
            <TestButton channel="telegram" disabled={dirty} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("notify.discord")}</CardTitle>
            <CardDescription>{t("notify.discordBody")}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <SwitchField label={t("notify.discordOn")} checked={form.discordEnabled} onCheckedChange={(discordEnabled) => set({ discordEnabled })} />
            <Field
              label={t("notify.discordUrl")}
              type="password"
              value={form.discordUrl}
              onChange={(e) => set({ discordUrl: e.target.value })}
              dir="ltr"
              autoComplete="off"
              placeholder={form.discordUrlSet ? t("notify.secretSet") : "https://discord.com/api/webhooks/…"}
            />
            <LastResult status={status.discord} />
            <TestButton channel="discord" disabled={dirty} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("notify.webhook")}</CardTitle>
            <CardDescription>{t("notify.webhookBody")}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <SwitchField label={t("notify.webhookOn")} checked={form.webhookEnabled} onCheckedChange={(webhookEnabled) => set({ webhookEnabled })} />
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label={t("notify.webhookUrl")} value={form.webhookUrl} onChange={(e) => set({ webhookUrl: e.target.value })} dir="ltr" placeholder="https://example.com/hook" />
              <Field
                label={t("notify.webhookSecret")}
                type="password"
                value={form.webhookSecret}
                onChange={(e) => set({ webhookSecret: e.target.value })}
                dir="ltr"
                autoComplete="off"
                placeholder={form.webhookSecretSet ? t("notify.secretSet") : ""}
                hint={t("notify.webhookSecretHint")}
              />
            </div>
            <LastResult status={status.webhook} />
            <TestButton channel="webhook" disabled={dirty} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("notify.events")}</CardTitle>
            <CardDescription>{t("notify.eventsBody")}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-3">
            {eventRows.map(({ key }) => (
              <SwitchField
                key={key}
                label={t(`notify.event_${key}`)}
                hint={t(`notify.event_${key}Hint`)}
                checked={form.events[key]}
                onCheckedChange={(on) => setEvent({ [key]: on })}
              />
            ))}
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("notify.dataLowPercent")}
                type="number"
                min={1}
                max={99}
                value={String(form.events.data_low_percent)}
                onChange={(e) => setEvent({ data_low_percent: Math.min(99, Math.max(1, Number(e.target.value) || 1)) })}
                dir="ltr"
                hint={t("notify.dataLowPercentHint")}
              />
              <Field
                label={t("notify.expiringDays")}
                type="number"
                min={1}
                max={60}
                value={String(form.events.expiring_days)}
                onChange={(e) => setEvent({ expiring_days: Math.min(60, Math.max(1, Number(e.target.value) || 1)) })}
                dir="ltr"
                hint={t("notify.expiringDaysHint")}
              />
            </div>
          </CardContent>
        </Card>
      </div>

      <Card className={dirty || save.isSuccess ? "sticky bottom-4 z-20 mt-4 shadow-lg" : "mt-4"}>
        <CardContent className="grid gap-2 p-4">
          {dirty ? (
            <div className="flex flex-wrap items-center justify-between gap-3">
              <span className="flex items-center gap-2 text-sm">
                <span className="size-2 rounded-full bg-warning" aria-hidden />
                {t("notify.unsaved")}
              </span>
              <div className="flex gap-2">
                <Button variant="ghost" size="sm" onClick={() => { setForm(fromView(view)); setProblem(null); save.reset(); }}>
                  <UndoIcon /> {t("core.discard")}
                </Button>
                <Button size="sm" disabled={save.isPending} onClick={submit}>
                  {t("settings.save")}
                </Button>
              </div>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{save.isSuccess ? t("settings.saved") : t("notify.nothingToSave")}</p>
          )}
          {problem && (
            <p role="alert" className="text-sm text-destructive">
              {problem}
            </p>
          )}
          <FormError error={save.error} />
        </CardContent>
      </Card>
    </>
  );
}
