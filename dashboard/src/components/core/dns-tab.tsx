import { PlusIcon } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import { TextareaField } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  dnsServerSummary,
  hasProxyOutbound,
  isSystemResolver,
  emptyDnsServer,
  fromDnsServer,
  hostsText,
  parseHosts,
  queryStrategies,
  toDnsServer,
  type DnsServerForm,
} from "@/lib/core-form";
import { listAt, move, objectAt, RowActions, type TabProps } from "./shared";

function ServerDialog({
  open,
  onOpenChange,
  server,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  server: unknown;
  onSave: (server: unknown) => void;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<DnsServerForm>(emptyDnsServer());
  const set = (patch: Partial<DnsServerForm>) => setForm((current) => ({ ...current, ...patch }));
  useEffect(() => {
    if (open) setForm(server === undefined ? emptyDnsServer() : fromDnsServer(server));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, server]);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onSave(toDnsServer(form, server));
    onOpenChange(false);
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={server === undefined ? t("core.newDnsServer") : t("core.editDnsServer")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <Field
            label={t("core.dnsAddress")}
            value={form.address}
            onChange={(e) => set({ address: e.target.value })}
            dir="ltr"
            required
            placeholder="https://1.1.1.1/dns-query"
            hint={t("core.dnsAddressHint")}
          />
          <div className="grid gap-4 sm:grid-cols-2">
            <TextareaField
              label={t("core.dnsDomains")}
              value={form.domains}
              onChange={(e) => set({ domains: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-20 font-mono text-xs"
              placeholder={"geosite:category-ir\ndomain:example.ir"}
              hint={t("core.dnsDomainsHint")}
            />
            <TextareaField
              label={t("core.dnsExpect")}
              value={form.expectIPs}
              onChange={(e) => set({ expectIPs: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-20 font-mono text-xs"
              placeholder="geoip:ir"
              hint={t("core.dnsExpectHint")}
            />
          </div>
          <Field label={t("core.tag")} value={form.tag} onChange={(e) => set({ tag: e.target.value })} dir="ltr" hint={t("core.dnsTagHint")} />
          <SwitchField label={t("core.dnsSkipFallback")} hint={t("core.dnsSkipFallbackHint")} checked={form.skipFallback} onCheckedChange={(skipFallback) => set({ skipFallback })} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit">{t("core.keepInEditor")}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function DnsTab({ settings, change }: TabProps) {
  const { t } = useTranslation();
  const dns = (settings.dns ?? {}) as Record<string, unknown>;
  const servers = Array.isArray(dns.servers) ? dns.servers : [];
  const [at, setAt] = useState<number | null>(null);
  const [open, setOpen] = useState(false);
  // Any outbound through another server makes the core refuse the system's
  // resolver, whatever the policy says.
  const forcedStrict = hasProxyOutbound(settings);
  const onlySystem = servers.every(isSystemResolver);
  // The hosts text is typed freely, so it is kept as typed until it is
  // taken in, and not rewritten under the cursor.
  const [hosts, setHosts] = useState(() => hostsText(dns.hosts));
  useEffect(() => setHosts(hostsText(dns.hosts)), [dns.hosts]);
  const setDns = (apply: (dns: Record<string, unknown>) => void) => change((draft) => apply(objectAt(draft, "dns")));

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("core.dns")}</CardTitle>
          <CardDescription>{t("core.dnsBody")}</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          {forcedStrict && (
            <div className="rounded-md border border-warning/40 bg-warning/10 p-3 text-sm">
              <p>{t("core.dnsForcedStrict")}</p>
              {onlySystem && <p className="mt-1 font-medium text-destructive">{t("core.dnsNoResolver")}</p>}
            </div>
          )}
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("core.queryStrategy")}
              value={String(dns.queryStrategy ?? "UseIP")}
              onChange={(e) => setDns((target) => void (target.queryStrategy = e.target.value))}
              options={queryStrategies.map((value) => ({ value, label: t(`core.queryStrategy_${value}`) }))}
              hint={t("core.queryStrategyHint")}
            />
            <SelectField
              label={t("core.leakPolicy")}
              value={forcedStrict || dns.leakPolicy !== "fallback" ? "strict" : "fallback"}
              disabled={forcedStrict}
              onChange={(e) =>
                setDns((target) => {
                  if (e.target.value === "fallback") target.leakPolicy = "fallback";
                  else delete target.leakPolicy;
                })
              }
              options={[
                { value: "fallback", label: t("core.leakFallback") },
                { value: "strict", label: t("core.leakStrict") },
              ]}
              hint={t("core.leakPolicyHint")}
            />
          </div>
          <SwitchField
            label={t("core.dnsDisableCache")}
            hint={t("core.dnsDisableCacheHint")}
            checked={dns.disableCache === true}
            onCheckedChange={(on) =>
              setDns((target) => {
                if (on) target.disableCache = true;
                else delete target.disableCache;
              })
            }
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle>{t("core.dnsServers")}</CardTitle>
            <CardDescription className="mt-1 max-w-2xl">{t("core.dnsServersBody")}</CardDescription>
          </div>
          <Button
            onClick={() => {
              setAt(null);
              setOpen(true);
            }}
          >
            <PlusIcon /> {t("core.addDnsServer")}
          </Button>
        </CardHeader>
        <CardContent className="p-0">
          {servers.length === 0 ? (
            <p className="p-6 text-center text-sm text-muted-foreground">{t("core.noDnsServers")}</p>
          ) : (
            <Table>
              <TableBody>
                {servers.map((server, index) => (
                  <TableRow key={index}>
                    <TableCell className="tabular w-10 text-muted-foreground">{index + 1}</TableCell>
                    <TableCell className="font-mono text-xs" dir="ltr">
                      {dnsServerSummary(server)}
                      {forcedStrict && isSystemResolver(server) && (
                        <Badge variant="destructive" className="ms-2 font-sans" dir="auto">
                          {t("core.dnsSystemUnused")}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      <RowActions
                        index={index}
                        count={servers.length}
                        onMove={(by) => setDns((target) => move(listAt(target, "servers"), index, by))}
                        onEdit={() => {
                          setAt(index);
                          setOpen(true);
                        }}
                        onDelete={() => setDns((target) => void listAt(target, "servers").splice(index, 1))}
                        deleteTitle={t("core.deleteDnsServer")}
                        deleteBody={t("core.deleteDnsServerBody")}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("core.dnsHosts")}</CardTitle>
          <CardDescription>{t("core.dnsHostsBody")}</CardDescription>
        </CardHeader>
        <CardContent>
          <TextareaField
            label={t("core.dnsHostsLabel")}
            value={hosts}
            onChange={(e) => setHosts(e.target.value)}
            onBlur={() =>
              setDns((target) => {
                const parsed = parseHosts(hosts);
                if (Object.keys(parsed).length) target.hosts = parsed;
                else delete target.hosts;
              })
            }
            dir="ltr"
            spellCheck={false}
            className="min-h-24 font-mono text-xs"
            placeholder={"example.com 1.2.3.4\nads.example.net #block"}
            hint={t("core.dnsHostsHint")}
          />
        </CardContent>
      </Card>

      <ServerDialog
        open={open}
        onOpenChange={setOpen}
        server={at === null ? undefined : servers[at]}
        onSave={(server) =>
          setDns((target) => {
            const list = listAt(target, "servers");
            if (at === null) list.push(server);
            else list[at] = server;
          })
        }
      />
    </div>
  );
}
