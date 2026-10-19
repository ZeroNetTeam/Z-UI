import { useMutation } from "@tanstack/react-query";
import { SparklesIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import { post, type CoreWarning, type IranOptions } from "@/lib/api";
import { canFragment, hasFragment, lines, outboundSummary, withCleanIp, withFragment, type Json } from "@/lib/core-form";
import type { TabProps } from "./shared";

const remoteDns = ["auto", "cloudflare", "google", "quad9", "adguard"];
const localDns = ["google", "cloudflare", "quad9", "system"];
const antiSanction = ["auto", "shecan", "403", "electro", "radar", "begzar", "none"];

/** What the preset dialog asks for, before it is split into lists. */
interface PresetForm {
  links: string;
  remote_dns: string;
  local_dns: string;
  anti_sanction_dns: string;
  block_ads: boolean;
  fragment: boolean;
  manage_geodata: boolean;
  clean_ip_candidates: string;
  clean_ip_host: string;
}

const presetDefaults: PresetForm = {
  links: "",
  remote_dns: "auto",
  local_dns: "google",
  anti_sanction_dns: "auto",
  block_ads: true,
  fragment: false,
  manage_geodata: true,
  clean_ip_candidates: "",
  clean_ip_host: "www.speedtest.net",
};

function PresetDialog({
  open,
  onOpenChange,
  settings,
  onPropose,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  settings: Json;
  onPropose: (settings: Json, warnings: CoreWarning[]) => void;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<PresetForm>(presetDefaults);
  const set = (patch: Partial<PresetForm>) => setForm((current) => ({ ...current, ...patch }));
  const propose = useMutation({
    mutationFn: () => {
      const options: IranOptions = {
        proxies: lines(form.links),
        remote_dns: form.remote_dns,
        local_dns: form.local_dns,
        anti_sanction_dns: form.anti_sanction_dns,
        block_ads: form.block_ads,
        fragment: form.fragment,
        manage_geodata: form.manage_geodata,
        clean_ip_candidates: lines(form.clean_ip_candidates),
        clean_ip_host: form.clean_ip_host.trim() || "www.speedtest.net",
      };
      return post<{ settings: Json; warnings: CoreWarning[] }>("core/presets/iran", { options, current: settings });
    },
    onSuccess: (proposed) => {
      onPropose(proposed.settings, proposed.warnings);
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    propose.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("core.presetTitle")} className="max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          <p className="text-sm text-muted-foreground">{t("core.presetBody")}</p>
          <TextareaField
            label={t("core.presetLinks")}
            value={form.links}
            onChange={(e) => set({ links: e.target.value })}
            dir="ltr"
            spellCheck={false}
            className="min-h-24 font-mono text-xs"
            placeholder={"vless://…\nvmess://…"}
            hint={t("core.presetLinksHint")}
            required
          />
          <div className="grid gap-4 sm:grid-cols-3">
            <SelectField
              label={t("core.presetRemote")}
              value={form.remote_dns}
              onChange={(e) => set({ remote_dns: e.target.value })}
              options={remoteDns.map((value) => ({ value, label: t(`core.dns_${value}`, { defaultValue: value }) }))}
            />
            <SelectField
              label={t("core.presetLocal")}
              value={form.local_dns}
              onChange={(e) => set({ local_dns: e.target.value })}
              options={localDns.map((value) => ({ value, label: t(`core.dns_${value}`, { defaultValue: value }) }))}
            />
            <SelectField
              label={t("core.presetSanction")}
              value={form.anti_sanction_dns}
              onChange={(e) => set({ anti_sanction_dns: e.target.value })}
              options={antiSanction.map((value) => ({ value, label: t(`core.dns_${value}`, { defaultValue: value }) }))}
            />
          </div>
          <div className="grid gap-3">
            <SwitchField label={t("core.presetBlockAds")} checked={form.block_ads} onCheckedChange={(block_ads) => set({ block_ads })} />
            <SwitchField label={t("core.fragment")} hint={t("core.fragmentHint")} checked={form.fragment} onCheckedChange={(fragment) => set({ fragment })} />
            <SwitchField label={t("core.geodataManage")} hint={t("core.geodataManageHint")} checked={form.manage_geodata} onCheckedChange={(manage_geodata) => set({ manage_geodata })} />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <TextareaField
              label={t("core.cleanIpCandidates")}
              value={form.clean_ip_candidates}
              onChange={(e) => set({ clean_ip_candidates: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-20 font-mono text-xs"
              placeholder={"104.16.0.1:443\n172.64.0.1:443"}
              hint={t("core.cleanIpCandidatesHint")}
            />
            <Field label={t("core.cleanIpHost")} value={form.clean_ip_host} onChange={(e) => set({ clean_ip_host: e.target.value })} dir="ltr" />
          </div>
          <FormError error={propose.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={propose.isPending}>
              {t("core.presetPropose")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function ZrayTab({ settings, change, onPropose }: TabProps & { onPropose: (settings: Json, warnings: CoreWarning[]) => void }) {
  const { t } = useTranslation();
  const [presetOpen, setPresetOpen] = useState(false);
  const outbounds = (Array.isArray(settings.outbounds) ? settings.outbounds : []) as Json[];
  const fragmentable = outbounds.map((outbound, index) => ({ outbound, index })).filter(({ outbound }) => canFragment(outbound));
  const clean = ((settings.observatory as Json | undefined)?.cleanIp ?? null) as Json | null;
  const candidates = Array.isArray(clean?.candidates) ? (clean?.candidates as string[]) : [];
  const [cleanText, setCleanText] = useState(candidates.join("\n"));
  const [cleanHost, setCleanHost] = useState(String(clean?.host ?? "www.speedtest.net"));

  const saveClean = (on: boolean, text = cleanText, host = cleanHost) =>
    change((draft) => {
      const next = withCleanIp(draft.observatory, on, lines(text), host);
      if (next) draft.observatory = next;
      else delete draft.observatory;
    });

  return (
    <div className="grid gap-4">
      <Card className="border-primary/30 bg-primary/5">
        <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle className="flex items-center gap-2">
              <SparklesIcon className="size-4 text-primary" /> {t("core.presetCard")}
            </CardTitle>
            <CardDescription className="mt-1 max-w-2xl">{t("core.presetCardBody")}</CardDescription>
          </div>
          <Button onClick={() => setPresetOpen(true)}>{t("core.presetOpen")}</Button>
        </CardHeader>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("core.fragmentTitle")}</CardTitle>
          <CardDescription>{t("core.fragmentBody")}</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-2">
          {fragmentable.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("core.fragmentNone")}</p>
          ) : (
            fragmentable.map(({ outbound, index }) => (
              <SwitchField
                key={`${String(outbound.tag)}-${index}`}
                label={String(outbound.tag ?? "")}
                hint={outboundSummary(outbound)}
                checked={hasFragment(outbound)}
                onCheckedChange={(on) => change((draft) => void ((draft.outbounds as Json[])[index] = withFragment(outbound, on)))}
              />
            ))
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {t("core.cleanIpTitle")} {clean && <Badge variant="success">{t("common.enabled")}</Badge>}
          </CardTitle>
          <CardDescription>{t("core.cleanIpBody")}</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          <SwitchField label={t("core.cleanIpOn")} checked={clean !== null} onCheckedChange={(on) => saveClean(on)} />
          {clean !== null && (
            <div className="grid gap-4 sm:grid-cols-2">
              <TextareaField
                label={t("core.cleanIpCandidates")}
                value={cleanText}
                onChange={(e) => setCleanText(e.target.value)}
                onBlur={() => saveClean(true)}
                dir="ltr"
                spellCheck={false}
                className="min-h-24 font-mono text-xs"
                placeholder={"104.16.0.1:443\n172.64.0.1:443"}
                hint={t("core.cleanIpCandidatesHint")}
              />
              <Field
                label={t("core.cleanIpHost")}
                value={cleanHost}
                onChange={(e) => setCleanHost(e.target.value)}
                onBlur={() => saveClean(true)}
                dir="ltr"
              />
            </div>
          )}
        </CardContent>
      </Card>

      <PresetDialog open={presetOpen} onOpenChange={setPresetOpen} settings={settings} onPropose={onPropose} />
    </div>
  );
}
