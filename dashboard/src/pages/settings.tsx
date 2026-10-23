import { useMutation } from "@tanstack/react-query";
import { CheckIcon, CopyIcon, ShieldCheckIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { PanelAccessCard } from "@/components/panel-access";
import { PageHeader } from "@/components/layout/layout";
import { SubscriptionSettingsCard } from "@/components/subscription-settings";
import { TelegramLinkCard } from "@/components/telegram-link";
import { UpdateCard } from "@/components/update-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { post, type Admin, type TotpSetup } from "@/lib/api";
import { useAuth, useAuthChanged } from "@/lib/auth";
import { can } from "@/lib/permissions";

function PasswordCard() {
  const { t } = useTranslation();
  const authChanged = useAuthChanged();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const change = useMutation({
    mutationFn: () => post<Admin>("auth/password", { current, new: next }),
    onSuccess: async () => {
      setCurrent("");
      setNext("");
      await authChanged();
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    change.mutate();
  };
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.passwordTitle")}</CardTitle>
        <CardDescription>{t("settings.passwordBody")}</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid max-w-sm gap-4">
          <Field
            label={t("settings.current")}
            type="password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            autoComplete="current-password"
            dir="ltr"
            required
          />
          <Field
            label={t("settings.new")}
            hint={t("auth.passwordHint")}
            type="password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            autoComplete="new-password"
            minLength={8}
            dir="ltr"
            required
          />
          <FormError error={change.error} />
          {change.isSuccess && <p className="text-sm text-success">{t("settings.saved")}</p>}
          <Button type="submit" className="justify-self-start" disabled={change.isPending}>
            {t("settings.save")}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? <CheckIcon /> : <CopyIcon />}
      {copied ? t("settings.copied") : t("settings.copy")}
    </Button>
  );
}

function TotpCard({ admin }: { admin: Admin }) {
  const { t } = useTranslation();
  const authChanged = useAuthChanged();
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");

  const begin = useMutation({
    mutationFn: () => post<TotpSetup>("auth/2fa/begin"),
    onSuccess: (data) => {
      setSetup(data);
      setCode("");
    },
  });
  const enable = useMutation({
    mutationFn: () => post<Admin>("auth/2fa/enable", { code }),
    onSuccess: async () => {
      setSetup(null);
      setCode("");
      await authChanged();
    },
  });
  const disable = useMutation({
    mutationFn: () => post<Admin>("auth/2fa/disable", { password, code }),
    onSuccess: async () => {
      setPassword("");
      setCode("");
      await authChanged();
    },
  });
  const codeField = (
    <Field
      label={t("auth.code")}
      value={code}
      onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
      inputMode="numeric"
      autoComplete="one-time-code"
      dir="ltr"
      className="tracking-[0.3em]"
      required
    />
  );

  return (
    <Card>
      <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
        <div className="grid gap-1.5">
          <CardTitle className="flex items-center gap-2">
            <ShieldCheckIcon className="size-4 text-primary" />
            {t("settings.totpTitle")}
          </CardTitle>
          <CardDescription>{t("settings.totpBody")}</CardDescription>
        </div>
        <Badge variant={admin.totp_enabled ? "success" : "secondary"}>
          {admin.totp_enabled ? t("settings.totpOn") : t("settings.totpOff")}
        </Badge>
      </CardHeader>
      <CardContent>
        {admin.totp_enabled ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              disable.mutate();
            }}
            className="grid max-w-sm gap-4"
          >
            <p className="text-sm text-muted-foreground">{t("settings.totpDisableBody")}</p>
            <Field
              label={t("auth.password")}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              dir="ltr"
              required
            />
            {codeField}
            <FormError error={disable.error} />
            <Button type="submit" variant="destructive" className="justify-self-start" disabled={disable.isPending}>
              {t("settings.totpDisable")}
            </Button>
          </form>
        ) : setup ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              enable.mutate();
            }}
            className="grid gap-4 sm:grid-cols-[auto_1fr] sm:items-start"
          >
            <img
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(setup.qr_svg)}`}
              alt="QR"
              className="size-44 rounded-lg bg-white p-2"
            />
            <div className="grid max-w-sm gap-4">
              <p className="text-sm text-muted-foreground">{t("settings.totpScan")}</p>
              <div className="grid gap-2">
                <span className="text-xs text-muted-foreground">{t("settings.totpSecret")}</span>
                <div className="flex items-center gap-2">
                  <code dir="ltr" className="truncate rounded bg-muted px-2 py-1 text-xs">
                    {setup.secret}
                  </code>
                  <CopyButton text={setup.secret} />
                </div>
              </div>
              {codeField}
              <FormError error={enable.error} />
              <div className="flex gap-2">
                <Button type="submit" disabled={enable.isPending || code.length !== 6}>
                  {t("settings.totpConfirm")}
                </Button>
                <Button type="button" variant="ghost" onClick={() => setSetup(null)}>
                  {t("settings.totpCancel")}
                </Button>
              </div>
            </div>
          </form>
        ) : (
          <div className="grid gap-3">
            <FormError error={begin.error} />
            <Button className="justify-self-start" onClick={() => begin.mutate()} disabled={begin.isPending}>
              {t("settings.totpStart")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function SettingsPage() {
  const { t } = useTranslation();
  const admin = useAuth().data?.admin;
  return (
    <>
      <PageHeader title={t("settings.title")} subtitle={t("settings.subtitle")} />
      <div className="grid gap-4">
        {can(admin, "settings.read") && (
          <fieldset disabled={!can(admin, "settings.write")} className="m-0 min-w-0 border-0 p-0">
            <SubscriptionSettingsCard />
          </fieldset>
        )}
        {can(admin, "sudo") && <PanelAccessCard />}
        {can(admin, "sudo") && <UpdateCard />}
        <PasswordCard />
        {admin && <TotpCard admin={admin} />}
        <TelegramLinkCard />
      </div>
    </>
  );
}
