import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import { api, put, type SubscriptionSettings } from "@/lib/api";

const variables = ["{remark}", "{username}", "{protocol}", "{transport}", "{security}", "{data_left}", "{days_left}", "{expire_date}", "{status}"];

/** Where subscriptions are served and what they say. */
export function SubscriptionSettingsCard() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const stored = useQuery({
    queryKey: ["settings", "subscription"],
    queryFn: () => api<SubscriptionSettings>("settings/subscription"),
  });
  const [form, setForm] = useState<SubscriptionSettings | null>(null);
  useEffect(() => {
    if (stored.data) setForm(stored.data);
  }, [stored.data]);
  const save = useMutation({
    mutationFn: (settings: SubscriptionSettings) => put<SubscriptionSettings>("settings/subscription", settings),
    onSuccess: (saved) => {
      client.setQueryData(["settings", "subscription"], saved);
      void client.invalidateQueries({ queryKey: ["users"] });
    },
  });
  const set = (patch: Partial<SubscriptionSettings>) => setForm((current) => (current ? { ...current, ...patch } : current));
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (form) save.mutate(form);
  };
  const example = `${form?.public_url || window.location.origin}${form?.path ?? "/sub"}/…`;
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("subscription.settingsTitle")}</CardTitle>
        <CardDescription>{t("subscription.settingsBody")}</CardDescription>
      </CardHeader>
      <CardContent>
        <FormError error={stored.error} />
        {form && (
          <form onSubmit={submit} className="grid gap-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("subscription.path")}
                value={form.path}
                onChange={(e) => set({ path: e.target.value })}
                dir="ltr"
                required
                hint={t("subscription.pathHint", { example })}
              />
              <Field
                label={t("subscription.publicUrl")}
                value={form.public_url}
                onChange={(e) => set({ public_url: e.target.value })}
                placeholder="https://sub.example.com"
                dir="ltr"
                hint={t("subscription.publicUrlHint")}
              />
              <Field
                label={t("subscription.serverAddress")}
                value={form.server_address}
                onChange={(e) => set({ server_address: e.target.value })}
                placeholder="vpn.example.com"
                dir="ltr"
                hint={t("subscription.serverAddressHint")}
              />
              <Field
                label={t("subscription.updateHours")}
                type="number"
                min={1}
                max={720}
                value={String(form.update_hours)}
                onChange={(e) => set({ update_hours: Number(e.target.value) || 1 })}
                dir="ltr"
              />
              <Field label={t("subscription.profileTitle")} value={form.title} onChange={(e) => set({ title: e.target.value })} />
              <Field
                label={t("subscription.supportUrl")}
                value={form.support_url}
                onChange={(e) => set({ support_url: e.target.value })}
                placeholder="https://t.me/…"
                dir="ltr"
              />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("subscription.legacyJsonPath")}
                value={form.legacy_json_path}
                onChange={(e) => set({ legacy_json_path: e.target.value })}
                placeholder="/json"
                dir="ltr"
                hint={t("subscription.legacyPathHint")}
              />
              <Field
                label={t("subscription.legacyClashPath")}
                value={form.legacy_clash_path}
                onChange={(e) => set({ legacy_clash_path: e.target.value })}
                placeholder="/clash"
                dir="ltr"
                hint={t("subscription.legacyPathHint")}
              />
            </div>
            <Field
              label={t("subscription.remarkTemplate")}
              value={form.remark_template}
              onChange={(e) => set({ remark_template: e.target.value })}
              dir="auto"
              hint={t("subscription.remarkHint", { variables: variables.join(" ") })}
            />
            <SwitchField
              label={t("subscription.hwidRequired")}
              hint={t("subscription.hwidRequiredHint")}
              checked={form.hwid_required}
              onCheckedChange={(hwid_required) => set({ hwid_required })}
            />
            <TextareaField
              label={t("subscription.template")}
              value={form.template}
              onChange={(e) => set({ template: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-24 font-mono text-xs"
              placeholder="<!doctype html>…<script>const sub = {{data}};</script>"
              hint={t("subscription.templateHint", { data: "{{data}}" })}
            />
            <FormError error={save.error} />
            {save.isSuccess && <p className="text-sm text-success">{t("settings.saved")}</p>}
            <Button type="submit" className="justify-self-start" disabled={save.isPending}>
              {t("settings.save")}
            </Button>
          </form>
        )}
      </CardContent>
    </Card>
  );
}
