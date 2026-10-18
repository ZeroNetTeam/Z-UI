import { useMutation, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2Icon, RefreshCwIcon, SearchCheckIcon, XCircleIcon } from "lucide-react";
import { useEffect, useId, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import {
  api,
  post,
  put,
  type Certificate,
  type Inbound,
  type InboundPayload,
  type RealityCheck,
} from "@/lib/api";
import {
  capabilities,
  defaults,
  fingerprints,
  fromInbound,
  protocols,
  shadowsocksKeyBytes,
  shadowsocksMethods,
  toPayload,
  visionPossible,
  xhttpModes,
  type InboundForm,
  type Protocol,
} from "@/lib/inbound-form";

/** An input with a button beside it that fills it (a key, an id). */
function KeyField({
  label,
  value,
  onChange,
  onGenerate,
  hint,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  onGenerate?: () => void;
  hint?: string;
}) {
  const { t } = useTranslation();
  const id = useId();
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex gap-2">
        <Input id={id} value={value} onChange={(e) => onChange(e.target.value)} dir="ltr" spellCheck={false} className="font-mono text-xs" />
        {onGenerate && (
          <Button type="button" variant="outline" size="icon" onClick={onGenerate} aria-label={t("common.generate")}>
            <RefreshCwIcon />
          </Button>
        )}
      </div>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

/** A bordered group of fields; its first field names it. */
function Section({ children }: { children: ReactNode }) {
  return <div className="grid gap-4 rounded-lg border p-4">{children}</div>;
}

async function realityKeys() {
  const [pair, short] = await Promise.all([
    api<{ private_key: string; public_key: string }>("keys/x25519"),
    api<{ short_id: string }>("keys/short-id?bytes=8"),
  ]);
  return { realityPrivateKey: pair.private_key, realityPublicKey: pair.public_key, realityShortIds: short.short_id };
}

/** Fill what a fresh form of this protocol needs generated. */
async function withKeys(form: InboundForm): Promise<InboundForm> {
  let next = { ...form };
  if (next.security === "reality" && !next.realityPrivateKey) next = { ...next, ...(await realityKeys()) };
  const bytes = shadowsocksKeyBytes(next.ssMethod);
  if (next.protocol === "shadowsocks" && bytes > 0 && !next.ssPassword) {
    next.ssPassword = (await api<{ password: string }>(`keys/password?bytes=${bytes}`)).password;
  }
  if (next.protocol === "wireguard" && !next.wgSecretKey) {
    const pair = await api<{ private_key: string; public_key: string }>("keys/wireguard");
    next.wgSecretKey = pair.private_key;
    next.wgPublicKey = pair.public_key;
  }
  return next;
}

function json(value: unknown) {
  return JSON.stringify(value, null, 2);
}

export function InboundDialog({
  open,
  onOpenChange,
  inbound,
  certificates,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  inbound?: Inbound;
  certificates: Certificate[];
  onSaved: (applied: string) => void;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [form, setForm] = useState<InboundForm>(defaults());
  const [jsonMode, setJsonMode] = useState(false);
  const [raw, setRaw] = useState({ settings: "{}", stream: "{}", sniffing: "{}" });
  const [jsonError, setJsonError] = useState(false);
  const [check, setCheck] = useState<RealityCheck | null>(null);

  useEffect(() => {
    if (!open) return;
    setJsonMode(false);
    setCheck(null);
    if (inbound) {
      setForm(fromInbound(inbound));
    } else {
      const fresh = defaults("vless");
      setForm(fresh);
      void withKeys(fresh).then(setForm);
    }
  }, [open, inbound]);

  const set = (patch: Partial<InboundForm>) => setForm((current) => ({ ...current, ...patch }));
  const caps = capabilities[form.protocol];

  const changeProtocol = (protocol: Protocol) => {
    const next = { ...defaults(protocol), remark: form.remark, port: form.port, listen: form.listen, enabled: form.enabled };
    setForm(next);
    void withKeys(next).then(setForm);
  };
  const changeSecurity = (security: InboundForm["security"]) => {
    const next = { ...form, security };
    setForm(next);
    if (security === "reality" && !form.realityPrivateKey) void withKeys(next).then(setForm);
  };

  const save = useMutation({
    mutationFn: async () => {
      let payload: InboundPayload = toPayload(form);
      if (jsonMode) {
        try {
          payload = { ...payload, settings: JSON.parse(raw.settings), stream: JSON.parse(raw.stream), sniffing: JSON.parse(raw.sniffing) };
        } catch {
          setJsonError(true);
          throw new Error("json");
        }
      }
      return inbound
        ? put<{ applied: string }>(`inbounds/${inbound.id}`, payload)
        : post<{ applied: string }>("inbounds", payload);
    },
    onSuccess: async (result) => {
      await client.invalidateQueries({ queryKey: ["inbounds"] });
      await client.invalidateQueries({ queryKey: ["health"] });
      onSaved(result.applied);
      onOpenChange(false);
    },
  });

  const realityCheck = useMutation({
    mutationFn: () => api<RealityCheck>(`tools/reality-check?target=${encodeURIComponent(form.realityTarget)}`),
    onSuccess: setCheck,
  });

  const toggleJson = () => {
    if (!jsonMode) {
      const payload = toPayload(form);
      setRaw({ settings: json(payload.settings), stream: json(payload.stream), sniffing: json(payload.sniffing) });
    }
    setJsonError(false);
    setJsonMode(!jsonMode);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setJsonError(false);
    save.mutate();
  };

  const certificateOptions = [
    { value: "", label: certificates.length ? "—" : t("inbounds.noCertificate") },
    ...certificates.map((c) => ({ value: String(c.id), label: `${c.name} (${c.domains.join(", ")})` })),
  ];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={inbound ? t("inbounds.editTitle") : t("inbounds.newTitle")} className="max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t("inbounds.remark")} value={form.remark} onChange={(e) => set({ remark: e.target.value })} required autoFocus />
            <SelectField
              label={t("inbounds.protocol")}
              value={form.protocol}
              onChange={(e) => changeProtocol(e.target.value as Protocol)}
              disabled={Boolean(inbound)}
              options={protocols.map((p) => ({ value: p, label: p }))}
            />
            <Field
              label={t("inbounds.port")}
              type="number"
              min={1}
              max={65535}
              value={form.port}
              onChange={(e) => set({ port: Number(e.target.value) })}
              dir="ltr"
              required
            />
            <Field
              label={t("inbounds.listen")}
              hint={t("inbounds.listenHint")}
              value={form.listen}
              onChange={(e) => set({ listen: e.target.value })}
              placeholder="0.0.0.0"
              dir="ltr"
            />
          </div>

          <div className="flex justify-end">
            <Button type="button" variant="ghost" size="sm" onClick={toggleJson}>
              {jsonMode ? t("inbounds.formMode") : t("inbounds.jsonMode")}
            </Button>
          </div>

          {jsonMode ? (
            <div className="grid gap-4">
              {(["settings", "stream", "sniffing"] as const).map((key) => (
                <TextareaField
                  key={key}
                  label={t(`inbounds.${key}Json`)}
                  value={raw[key]}
                  onChange={(e) => setRaw({ ...raw, [key]: e.target.value })}
                  dir="ltr"
                  spellCheck={false}
                  className="min-h-32 font-mono text-xs"
                />
              ))}
              {jsonError && <p className="text-sm text-destructive">{t("inbounds.invalidJson")}</p>}
            </div>
          ) : (
            <>
              {form.protocol === "shadowsocks" && (
                <Section>
                  <SelectField
                    label={t("inbounds.method")}
                    value={form.ssMethod}
                    onChange={(e) => {
                      const next = { ...form, ssMethod: e.target.value, ssPassword: "" };
                      setForm(next);
                      void withKeys(next).then(setForm);
                    }}
                    options={shadowsocksMethods.map((m) => ({ value: m, label: m }))}
                  />
                  {shadowsocksKeyBytes(form.ssMethod) > 0 && (
                    <KeyField
                      label={t("inbounds.serverKey")}
                      value={form.ssPassword}
                      onChange={(ssPassword) => set({ ssPassword })}
                      onGenerate={() => void withKeys({ ...form, ssPassword: "" }).then(setForm)}
                    />
                  )}
                </Section>
              )}

              {form.protocol === "wireguard" && (
                <Section>
                  <KeyField
                    label={t("inbounds.secretKey")}
                    value={form.wgSecretKey}
                    onChange={(wgSecretKey) => set({ wgSecretKey })}
                    onGenerate={() => void withKeys({ ...form, wgSecretKey: "" }).then(setForm)}
                  />
                  <KeyField label={t("inbounds.publicKey")} value={form.wgPublicKey} onChange={(wgPublicKey) => set({ wgPublicKey })} />
                  <Field label={t("inbounds.mtu")} type="number" value={form.wgMtu} onChange={(e) => set({ wgMtu: Number(e.target.value) })} dir="ltr" />
                </Section>
              )}

              {form.protocol === "hysteria2" && (
                <Field
                  label={t("inbounds.obfs")}
                  hint={t("inbounds.obfsHint")}
                  value={form.obfsPassword}
                  onChange={(e) => set({ obfsPassword: e.target.value })}
                  dir="ltr"
                />
              )}

              {caps.transports.length > 1 && (
                <Section>
                  <SelectField
                    label={t("inbounds.transport")}
                    value={form.transport}
                    onChange={(e) => set({ transport: e.target.value as InboundForm["transport"] })}
                    options={caps.transports.map((v) => ({ value: v, label: v }))}
                  />
                  {["ws", "httpupgrade", "xhttp"].includes(form.transport) && (
                    <div className="grid gap-4 sm:grid-cols-2">
                      <Field label={t("inbounds.path")} value={form.path} onChange={(e) => set({ path: e.target.value })} dir="ltr" />
                      <Field label={t("inbounds.host")} value={form.host} onChange={(e) => set({ host: e.target.value })} dir="ltr" />
                    </div>
                  )}
                  {form.transport === "grpc" && (
                    <Field label={t("inbounds.serviceName")} value={form.serviceName} onChange={(e) => set({ serviceName: e.target.value })} dir="ltr" />
                  )}
                  {form.transport === "xhttp" && (
                    <SelectField
                      label={t("inbounds.xhttpMode")}
                      value={form.xhttpMode}
                      onChange={(e) => set({ xhttpMode: e.target.value as InboundForm["xhttpMode"] })}
                      options={xhttpModes.map((v) => ({ value: v, label: v }))}
                    />
                  )}
                </Section>
              )}

              {caps.securities.length > 1 || caps.securities[0] !== "none" ? (
                <Section>
                  {caps.securities.length > 1 && (
                    <SelectField
                      label={t("inbounds.security")}
                      value={form.security}
                      onChange={(e) => changeSecurity(e.target.value as InboundForm["security"])}
                      options={caps.securities.map((v) => ({ value: v, label: v === "none" ? t("common.none") : v.toUpperCase() }))}
                    />
                  )}
                  {form.security === "tls" && (
                    <div className="grid gap-4 sm:grid-cols-2">
                      <SelectField
                        label={t("inbounds.certificate")}
                        value={form.certificateId === null ? "" : String(form.certificateId)}
                        onChange={(e) => set({ certificateId: e.target.value ? Number(e.target.value) : null })}
                        options={certificateOptions}
                        required
                      />
                      <Field label={t("inbounds.alpn")} value={form.alpn} onChange={(e) => set({ alpn: e.target.value })} dir="ltr" />
                    </div>
                  )}
                  {form.security === "reality" && (
                    <div className="grid gap-4">
                      <div className="grid gap-2">
                        <Label htmlFor="reality-target">{t("inbounds.target")}</Label>
                        <div className="flex flex-col gap-2 sm:flex-row">
                          <Input
                            id="reality-target"
                            value={form.realityTarget}
                            onChange={(e) => {
                              const target = e.target.value;
                              setCheck(null);
                              set({ realityTarget: target, realityServerNames: target.split(":")[0] });
                            }}
                            dir="ltr"
                          />
                          <Button
                            type="button"
                            variant="outline"
                            onClick={() => realityCheck.mutate()}
                            disabled={realityCheck.isPending || !form.realityTarget}
                          >
                            <SearchCheckIcon /> {t("inbounds.checkTarget")}
                          </Button>
                        </div>
                        <p className="text-xs text-muted-foreground">{t("inbounds.targetHint")}</p>
                        <FormError error={realityCheck.error} />
                        {check && (
                          <div className="flex flex-wrap items-center gap-1.5 text-xs" dir="ltr">
                            <Badge variant={check.suitable ? "success" : "destructive"}>
                              {check.suitable ? <CheckCircle2Icon /> : <XCircleIcon />}
                              {check.suitable ? t("inbounds.suitable") : t("inbounds.unsuitable")}
                            </Badge>
                            {(["tls13", "h2", "x25519", "certificate_valid"] as const).map((key) => (
                              <Badge key={key} variant={check[key] ? "secondary" : "destructive"}>
                                {key === "tls13" ? "TLS 1.3" : key === "certificate_valid" ? "cert" : key}
                              </Badge>
                            ))}
                            {check.latency_ms !== null && <span className="text-muted-foreground">{check.latency_ms} ms</span>}
                            {check.error && <span className="text-destructive">{check.error}</span>}
                          </div>
                        )}
                      </div>
                      <Field
                        label={t("inbounds.serverNames")}
                        value={form.realityServerNames}
                        onChange={(e) => set({ realityServerNames: e.target.value })}
                        dir="ltr"
                      />
                      <KeyField
                        label={t("inbounds.privateKey")}
                        value={form.realityPrivateKey}
                        onChange={(realityPrivateKey) => set({ realityPrivateKey })}
                        onGenerate={() => void realityKeys().then((keys) => set({ realityPrivateKey: keys.realityPrivateKey, realityPublicKey: keys.realityPublicKey }))}
                      />
                      <KeyField label={t("inbounds.publicKey")} value={form.realityPublicKey} onChange={(realityPublicKey) => set({ realityPublicKey })} />
                      <div className="grid gap-4 sm:grid-cols-2">
                        <KeyField
                          label={t("inbounds.shortIds")}
                          value={form.realityShortIds}
                          onChange={(realityShortIds) => set({ realityShortIds })}
                          onGenerate={() =>
                            void api<{ short_id: string }>("keys/short-id?bytes=8").then((r) => set({ realityShortIds: r.short_id }))
                          }
                        />
                        <SelectField
                          label={t("inbounds.fingerprint")}
                          value={form.fingerprint}
                          onChange={(e) => set({ fingerprint: e.target.value })}
                          options={fingerprints.map((v) => ({ value: v, label: v }))}
                        />
                      </div>
                    </div>
                  )}
                  {visionPossible(form) && (
                    <SelectField
                      label={t("inbounds.flow")}
                      value={form.flow}
                      onChange={(e) => set({ flow: e.target.value as InboundForm["flow"] })}
                      options={[
                        { value: "xtls-rprx-vision", label: t("inbounds.vision") },
                        { value: "", label: t("inbounds.noFlow") },
                      ]}
                    />
                  )}
                </Section>
              ) : null}

              <SwitchField
                label={t("inbounds.sniffing")}
                hint={t("inbounds.sniffingHint")}
                checked={form.sniffing}
                onCheckedChange={(sniffing) => set({ sniffing })}
              />
            </>
          )}

          <FormError error={save.error instanceof Error && save.error.message === "json" ? null : save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {inbound ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
