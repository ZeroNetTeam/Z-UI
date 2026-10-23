import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2Icon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import { api, post, put, type AcmeCa, type AcmeInput, type AcmeView, type Certificate, type ChallengeMethod } from "@/lib/api";

export const acmeKey = ["acme"] as const;

export function useAcme() {
  return useQuery({ queryKey: acmeKey, queryFn: () => api<AcmeView>("certificates/acme") });
}

type Props = { open: boolean; onOpenChange: (open: boolean) => void };

/** Whether `names` (as typed) hold a wildcard. */
export function hasWildcard(names: string): boolean {
  return names.split(/[\s,]+/).some((name) => name.startsWith("*."));
}

export function IssueCertificateDialog({ open, onOpenChange }: Props) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const acme = useAcme();
  const [names, setNames] = useState("");
  const [name, setName] = useState("");
  const [method, setMethod] = useState<ChallengeMethod>("auto");
  const [autoRenew, setAutoRenew] = useState(true);
  const issue = useMutation({
    mutationFn: () => post<Certificate>("certificates/issue", { name, names: names.split(/[\s,]+/).filter(Boolean), method, auto_renew: autoRenew }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["certificates"] });
      setNames("");
      setName("");
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    issue.mutate();
  };
  const usesDns = method === "dns-01" || hasWildcard(names);
  const noToken = usesDns && acme.data && !acme.data.cloudflare_token_set;
  const hints: Record<ChallengeMethod, string> = {
    auto: t("certificates.methodAutoHint"),
    "http-01": t("certificates.methodHttpHint"),
    "tls-alpn-01": t("certificates.methodTlsHint"),
    "dns-01": t("certificates.methodDnsHint"),
  };
  return (
    <Dialog open={open} onOpenChange={(next) => !issue.isPending && onOpenChange(next)}>
      <DialogContent title={t("certificates.issueTitle")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <p className="text-sm text-muted-foreground">{t("certificates.issueBody")}</p>
          <TextareaField
            label={t("certificates.names")}
            hint={t("certificates.namesHint")}
            value={names}
            onChange={(e) => setNames(e.target.value)}
            placeholder={"panel.example.com\n*.example.com"}
            dir="ltr"
            spellCheck={false}
            className="min-h-20 font-mono text-sm"
            required
          />
          <SelectField
            label={t("certificates.method")}
            hint={hints[method]}
            value={method}
            onChange={(e) => setMethod(e.target.value as ChallengeMethod)}
            options={(["auto", "http-01", "tls-alpn-01", "dns-01"] as const).map((value) => ({ value, label: t(`certificates.method_${value}`) }))}
          />
          {noToken && (
            <p role="status" className="rounded-md border border-primary/40 bg-primary/5 px-3 py-2 text-sm">
              {t("certificates.needCloudflare")}
            </p>
          )}
          <Field label={t("certificates.certificateName")} value={name} onChange={(e) => setName(e.target.value)} />
          <SwitchField label={t("certificates.autoRenew")} hint={t("certificates.autoRenewHint")} checked={autoRenew} onCheckedChange={setAutoRenew} />
          <FormError error={issue.error} />
          {issue.isPending && (
            <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2Icon className="size-4 animate-spin" /> {t("certificates.issuing")}
            </p>
          )}
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" disabled={issue.isPending} onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={issue.isPending || !names.trim()}>
              {t("certificates.issue")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SelfSignedDialog({ open, onOpenChange }: Props) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [names, setNames] = useState("");
  const [name, setName] = useState("");
  const make = useMutation({
    mutationFn: () => post<Certificate>("certificates/self-signed", { name, names: names.split(/[\s,]+/).filter(Boolean) }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["certificates"] });
      setNames("");
      setName("");
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    make.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("certificates.selfSignedTitle")} className="max-w-lg">
        <form onSubmit={submit} className="grid gap-4">
          <p className="text-sm text-muted-foreground">{t("certificates.selfSignedBody")}</p>
          <TextareaField
            label={t("certificates.names")}
            hint={t("certificates.namesHint")}
            value={names}
            onChange={(e) => setNames(e.target.value)}
            placeholder={"203.0.113.7"}
            dir="ltr"
            spellCheck={false}
            className="min-h-16 font-mono text-sm"
            required
          />
          <Field label={t("certificates.certificateName")} value={name} onChange={(e) => setName(e.target.value)} />
          <FormError error={make.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={make.isPending || !names.trim()}>
              {t("certificates.make")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** What the form holds: the stored settings, with each secret empty until typed. */
function fromView(view: AcmeView) {
  return {
    ca: view.ca,
    directory_url: view.directory_url,
    email: view.email,
    eab_kid: view.eab_kid,
    eab_hmac: "",
    clear_eab: false,
    cloudflare_token: "",
    clear_cloudflare: false,
    http_port: String(view.http_port),
    tls_port: String(view.tls_port),
    renew_days: String(view.renew_days),
    proxy: view.proxy,
    dns_wait: String(view.dns_wait),
  };
}

function AcmeForm({ view, onDone }: { view: AcmeView; onDone: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [form, setForm] = useState(() => fromView(view));
  const set = <K extends keyof typeof form>(key: K, value: (typeof form)[K]) => setForm((f) => ({ ...f, [key]: value }));
  const save = useMutation({
    mutationFn: () => {
      const payload: AcmeInput = {
        ...form,
        http_port: Number(form.http_port),
        tls_port: Number(form.tls_port),
        renew_days: Number(form.renew_days),
        dns_wait: Number(form.dns_wait),
      };
      return put<AcmeView>("certificates/acme", payload);
    },
    onSuccess: async (saved) => {
      client.setQueryData(acmeKey, saved);
      await client.invalidateQueries({ queryKey: ["certificates"] });
      onDone();
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  const wantsKey = form.ca === "zerossl" || form.ca === "custom";
  return (
    <form onSubmit={submit} className="grid gap-4">
      <p className="text-sm text-muted-foreground">{t("acme.subtitle")}</p>
      <SelectField
        label={t("acme.ca")}
        value={form.ca}
        onChange={(e) => set("ca", e.target.value as AcmeCa)}
        options={(["letsencrypt", "letsencrypt_staging", "zerossl", "custom"] as const).map((value) => ({ value, label: t(`acme.ca_${value}`) }))}
      />
      {form.ca === "custom" && (
        <Field label={t("acme.directory")} value={form.directory_url} onChange={(e) => set("directory_url", e.target.value)} dir="ltr" placeholder="https://ca.example/directory" required />
      )}
      <Field label={t("acme.email")} hint={t("acme.emailHint")} type="email" value={form.email} onChange={(e) => set("email", e.target.value)} dir="ltr" />
      {wantsKey && (
        <div className="grid gap-3 rounded-md border p-3">
          <p className="text-xs text-muted-foreground">{t("acme.eabHint")}</p>
          <Field label={t("acme.eabKid")} value={form.eab_kid} onChange={(e) => set("eab_kid", e.target.value)} dir="ltr" />
          <Field
            label={t("acme.eabHmac")}
            hint={view.eab_hmac_set ? t("acme.secretSet") : undefined}
            type="password"
            value={form.eab_hmac}
            onChange={(e) => set("eab_hmac", e.target.value)}
            autoComplete="off"
            dir="ltr"
          />
          {(view.eab_hmac_set || view.eab_kid) && (
            <SwitchField label={t("acme.clearEab")} checked={form.clear_eab} onCheckedChange={(on) => set("clear_eab", on)} />
          )}
        </div>
      )}
      <div className="grid gap-3 rounded-md border p-3">
        <Field
          label={t("acme.cloudflare")}
          hint={view.cloudflare_token_set ? `${t("acme.secretSet")} · ${t("acme.cloudflareHint")}` : t("acme.cloudflareHint")}
          type="password"
          value={form.cloudflare_token}
          onChange={(e) => set("cloudflare_token", e.target.value)}
          autoComplete="off"
          dir="ltr"
        />
        {view.cloudflare_token_set && (
          <SwitchField label={t("acme.clearCloudflare")} checked={form.clear_cloudflare} onCheckedChange={(on) => set("clear_cloudflare", on)} />
        )}
        <Field label={t("acme.dnsWait")} type="number" min={0} max={300} value={form.dns_wait} onChange={(e) => set("dns_wait", e.target.value)} dir="ltr" />
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={t("acme.httpPort")} hint={t("acme.httpPortHint")} type="number" min={1} max={65535} value={form.http_port} onChange={(e) => set("http_port", e.target.value)} dir="ltr" />
        <Field label={t("acme.tlsPort")} type="number" min={1} max={65535} value={form.tls_port} onChange={(e) => set("tls_port", e.target.value)} dir="ltr" />
      </div>
      <Field label={t("acme.renewDays")} hint={t("acme.renewDaysHint")} type="number" min={1} max={60} value={form.renew_days} onChange={(e) => set("renew_days", e.target.value)} dir="ltr" />
      <Field label={t("acme.proxy")} hint={t("acme.proxyHint")} value={form.proxy} onChange={(e) => set("proxy", e.target.value)} dir="ltr" placeholder="socks5://127.0.0.1:10808" />
      <p className="text-xs text-muted-foreground">
        {t("acme.account")}: {view.account_ready ? t("acme.accountReady") : t("acme.accountNone")}
      </p>
      <FormError error={save.error} />
      <div className="flex justify-end gap-2 border-t pt-4">
        <Button type="button" variant="ghost" onClick={onDone}>
          {t("common.cancel")}
        </Button>
        <Button type="submit" disabled={save.isPending}>
          {t("common.save")}
        </Button>
      </div>
    </form>
  );
}

export function AcmeSettingsDialog({ open, onOpenChange }: Props) {
  const { t } = useTranslation();
  const acme = useAcme();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("acme.title")} className="max-w-xl">
        {acme.data ? <AcmeForm key={JSON.stringify(acme.data)} view={acme.data} onDone={() => onOpenChange(false)} /> : <FormError error={acme.error} />}
      </DialogContent>
    </Dialog>
  );
}
