import { useMutation, useQueryClient } from "@tanstack/react-query";
import { PlusIcon, RefreshCwIcon } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { TextareaField } from "@/components/ui/textarea";
import { post } from "@/lib/api";
import {
  balancerStrategies,
  emptyBalancer,
  emptyRule,
  fromBalancer,
  fromObservatory,
  fromRule,
  ruleConditions,
  ruleTarget,
  sniffedProtocols,
  usesGeodata,
  toBalancer,
  toObservatory,
  toRule,
  type BalancerForm,
  type Json,
  type RuleForm,
} from "@/lib/core-form";
import { formatDuration } from "@/lib/utils";
import { Checks, listAt, move, objectAt, RowActions, type TabProps } from "./shared";

const tagsOf = (list: unknown): string[] =>
  (Array.isArray(list) ? list : []).map((item) => String((item as Json).tag ?? "")).filter(Boolean);

function RuleDialog({
  open,
  onOpenChange,
  rule,
  settings,
  inbounds,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rule: Json | null;
  settings: Json;
  inbounds: { tag: string; remark: string }[];
  onSave: (rule: Json) => void;
}) {
  const { t } = useTranslation();
  const outbounds = tagsOf(settings.outbounds);
  const balancers = tagsOf((settings.routing as Json | undefined)?.balancers);
  const [form, setForm] = useState<RuleForm>(emptyRule());
  const set = (patch: Partial<RuleForm>) => setForm((current) => ({ ...current, ...patch }));

  useEffect(() => {
    if (open) setForm(rule ? fromRule(rule) : emptyRule(outbounds[0] ?? ""));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, rule]);

  const targets = form.targetKind === "balancer" ? balancers : outbounds;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onSave(toRule(form, rule ?? undefined));
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={rule ? t("core.editRule") : t("core.newRule")} className="max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          <p className="text-sm text-muted-foreground">{t("core.ruleHint")}</p>
          <div className="grid gap-4 sm:grid-cols-2">
            <TextareaField
              label={t("core.ruleDomain")}
              value={form.domain}
              onChange={(e) => set({ domain: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-24 font-mono text-xs"
              placeholder={"geosite:category-ir\ndomain:example.com\nfull:www.example.com\nregexp:\\.ir$\nkeyword:bank"}
              hint={t("core.ruleDomainHint")}
            />
            <TextareaField
              label={t("core.ruleIp")}
              value={form.ip}
              onChange={(e) => set({ ip: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-24 font-mono text-xs"
              placeholder={"geoip:ir\ngeoip:private\n1.2.3.0/24"}
              hint={t("core.ruleIpHint")}
            />
            <Field label={t("core.rulePort")} value={form.port} onChange={(e) => set({ port: e.target.value })} dir="ltr" placeholder="443, 1000-2000" />
            <SelectField
              label={t("core.ruleNetwork")}
              value={form.network}
              onChange={(e) => set({ network: e.target.value })}
              options={[
                { value: "", label: t("core.ruleAnyNetwork") },
                { value: "tcp", label: "TCP" },
                { value: "udp", label: "UDP" },
              ]}
            />
          </div>
          <Checks
            label={t("core.ruleInbounds")}
            options={inbounds.map((inbound) => ({ value: inbound.tag, label: `${inbound.remark || inbound.tag} (${inbound.tag})` }))}
            value={form.inboundTag}
            onChange={(inboundTag) => set({ inboundTag })}
            empty={t("core.ruleNoInbounds")}
          />
          <Checks
            label={t("core.ruleProtocols")}
            options={sniffedProtocols.map((protocol) => ({ value: protocol, label: protocol }))}
            value={form.protocol}
            onChange={(protocol) => set({ protocol })}
          />
          <Field label={t("core.ruleUsers")} value={form.user} onChange={(e) => set({ user: e.target.value })} dir="ltr" placeholder="alice, bob" hint={t("core.ruleUsersHint")} />
          <div className="grid gap-4 rounded-lg border p-4 sm:grid-cols-2">
            <SelectField
              label={t("core.ruleSendTo")}
              value={form.targetKind}
              onChange={(e) => {
                const targetKind = e.target.value as RuleForm["targetKind"];
                set({ targetKind, target: (targetKind === "balancer" ? balancers : outbounds)[0] ?? "" });
              }}
              options={[
                { value: "outbound", label: t("core.ruleToOutbound") },
                { value: "balancer", label: t("core.ruleToBalancer") },
              ]}
            />
            <SelectField
              label={form.targetKind === "balancer" ? t("core.balancer") : t("core.outbound")}
              value={form.target}
              onChange={(e) => set({ target: e.target.value })}
              options={targets.map((tag) => ({ value: tag, label: tag }))}
              required
            />
          </div>
          {ruleConditions(toRule(form)).length === 0 && <p className="text-xs text-warning">{t("core.ruleMatchesAll")}</p>}
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={!form.target}>
              {t("core.keepInEditor")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function BalancerDialog({
  open,
  onOpenChange,
  balancer,
  settings,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  balancer: Json | null;
  settings: Json;
  onSave: (balancer: Json) => void;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<BalancerForm>(emptyBalancer());
  const [problem, setProblem] = useState<string | null>(null);
  const set = (patch: Partial<BalancerForm>) => setForm((current) => ({ ...current, ...patch }));
  const outbounds = tagsOf(settings.outbounds);
  const taken = tagsOf((settings.routing as Json | undefined)?.balancers).filter((tag) => tag !== balancer?.tag);

  useEffect(() => {
    if (open) {
      setForm(balancer ? fromBalancer(balancer) : emptyBalancer());
      setProblem(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, balancer]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const tag = form.tag.trim();
    if (!tag) return setProblem(t("core.tagRequired"));
    if (taken.includes(tag) || outbounds.includes(tag)) return setProblem(t("core.tagTaken"));
    onSave(toBalancer(form, balancer ?? undefined));
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={balancer ? t("core.editBalancer") : t("core.newBalancer")}>
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("core.tag")} value={form.tag} onChange={(e) => set({ tag: e.target.value })} dir="ltr" required />
          <TextareaField
            label={t("core.balancerSelector")}
            value={form.selector}
            onChange={(e) => set({ selector: e.target.value })}
            dir="ltr"
            spellCheck={false}
            className="min-h-20 font-mono text-xs"
            placeholder={outbounds.slice(0, 2).join("\n")}
            hint={t("core.balancerSelectorHint")}
            required
          />
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("core.balancerStrategy")}
              value={form.strategy}
              onChange={(e) => set({ strategy: e.target.value })}
              options={balancerStrategies.map((value) => ({ value, label: t(`core.strategy_${value}`) }))}
              hint={form.strategy.startsWith("least") ? t("core.strategyNeedsObservatory") : undefined}
            />
            <SelectField
              label={t("core.balancerFallback")}
              value={form.fallbackTag}
              onChange={(e) => set({ fallbackTag: e.target.value })}
              options={[{ value: "", label: t("common.none") }, ...outbounds.map((tag) => ({ value: tag, label: tag }))]}
            />
          </div>
          {problem && (
            <p role="alert" className="text-sm text-destructive">
              {problem}
            </p>
          )}
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

function Geodata({ settings, change, core }: TabProps) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const managed = settings.assets !== undefined && settings.assets !== null;
  const refresh = useMutation({
    mutationFn: () => post<Record<string, unknown>>("core/geodata/refresh"),
    onSuccess: () => client.invalidateQueries({ queryKey: ["core"] }),
  });
  const data = core.geodata;
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("core.geodata")}</CardTitle>
        <CardDescription>{t("core.geodataBody")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        <SwitchField
          label={t("core.geodataManage")}
          hint={t("core.geodataManageHint")}
          checked={managed}
          onCheckedChange={(on) =>
            change((draft) => {
              if (on) draft.assets = { refreshInterval: "24h", retryInterval: "15m", refreshOnStart: true, files: core.asset_files };
              else delete draft.assets;
            })
          }
        />
        {managed && (
          <div className="grid gap-3">
            {data ? (
              <>
                <div className="flex flex-wrap items-center justify-between gap-2 text-sm text-muted-foreground">
                  <span>{t("core.geodataTags", { sites: data.geositeTags, ips: data.geoipTags })}</span>
                  <Button variant="outline" size="sm" disabled={refresh.isPending} onClick={() => refresh.mutate()}>
                    <RefreshCwIcon className={refresh.isPending ? "animate-spin" : undefined} /> {t("core.geodataRefresh")}
                  </Button>
                </div>
                {data.files.length > 0 && (
                  <ul className="grid gap-1 text-sm">
                    {data.files.map((file) => (
                      <li key={file.name} className="flex flex-wrap items-center justify-between gap-2 rounded-md border px-3 py-2">
                        <span className="font-mono text-xs" dir="ltr">
                          {file.name}
                        </span>
                        <span className="flex items-center gap-2 text-xs text-muted-foreground">
                          <Badge variant={file.outcome === "failed" ? "destructive" : "success"}>{file.outcome}</Badge>
                          {file.atUnixSeconds > 0 && <span>{t("core.geodataAge", { age: formatDuration(Math.max(0, Date.now() / 1000 - file.atUnixSeconds), i18n.language) })}</span>}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                <FormError error={refresh.error} />
              </>
            ) : (
              <p className="text-sm text-muted-foreground">{t("core.geodataApplyFirst")}</p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function RoutingTab(props: TabProps) {
  const { settings, change, core } = props;
  const { t } = useTranslation();
  const routing = (settings.routing ?? {}) as Json;
  const rules = (Array.isArray(routing.rules) ? routing.rules : []) as Json[];
  const balancers = (Array.isArray(routing.balancers) ? routing.balancers : []) as Json[];
  const observatory = fromObservatory(settings.observatory);
  const [ruleAt, setRuleAt] = useState<number | null>(null);
  const [ruleOpen, setRuleOpen] = useState(false);
  const [balancerAt, setBalancerAt] = useState<number | null>(null);
  const [balancerOpen, setBalancerOpen] = useState(false);
  const setObservatory = (patch: Partial<typeof observatory>) =>
    change((draft) => {
      const next = toObservatory({ ...observatory, ...patch }, draft.observatory);
      if (next) draft.observatory = next;
      else delete draft.observatory;
    });

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("core.routing")}</CardTitle>
          <CardDescription>{t("core.routingBody")}</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("core.routingStrategy")}
              value={String(routing.domainStrategy ?? "AsIs")}
              onChange={(e) =>
                change((draft) => {
                  const target = objectAt(draft, "routing");
                  if (e.target.value === "AsIs") delete target.domainStrategy;
                  else target.domainStrategy = e.target.value;
                })
              }
              options={["AsIs", "IPIfNonMatch", "IPOnDemand"].map((value) => ({ value, label: t(`core.routingStrategy_${value}`) }))}
              hint={t("core.routingStrategyHint")}
            />
          </div>
          <SwitchField
            label={t("core.blockPrivate")}
            hint={t("core.blockPrivateHint")}
            checked={settings.blockPrivate !== false}
            onCheckedChange={(on) => change((draft) => void (draft.blockPrivate = on))}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle>{t("core.rules")}</CardTitle>
            <CardDescription className="mt-1 max-w-2xl">{t("core.rulesBody")}</CardDescription>
          </div>
          <Button
            onClick={() => {
              setRuleAt(null);
              setRuleOpen(true);
            }}
          >
            <PlusIcon /> {t("core.addRule")}
          </Button>
        </CardHeader>
        {usesGeodata(settings) && settings.assets == null && (core.geodata?.geositeTags ?? 0) + (core.geodata?.geoipTags ?? 0) === 0 && (
          <div className="mx-6 mb-4 flex flex-wrap items-center justify-between gap-3 rounded-md border border-warning/40 bg-warning/10 p-3 text-sm">
            <span>{t("core.geodataNeeded")}</span>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                change((draft) => {
                  draft.assets = { refreshInterval: "24h", retryInterval: "15m", refreshOnStart: true, files: core.asset_files };
                })
              }
            >
              {t("core.geodataTurnOn")}
            </Button>
          </div>
        )}
        <CardContent className="p-0">
          {rules.length === 0 ? (
            <p className="p-6 text-center text-sm text-muted-foreground">{t("core.noRules")}</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-10">#</TableHead>
                  <TableHead>{t("core.ruleMatches")}</TableHead>
                  <TableHead>{t("core.ruleGoesTo")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rules.map((rule, index) => {
                  const conditions = ruleConditions(rule);
                  return (
                    <TableRow key={index}>
                      <TableCell className="tabular text-muted-foreground">{index + 1}</TableCell>
                      <TableCell className="max-w-md">
                        <div className="flex flex-wrap gap-1" dir="ltr">
                          {conditions.length === 0 ? (
                            <span className="text-xs text-muted-foreground">{t("core.ruleAll")}</span>
                          ) : (
                            conditions.map((condition) => (
                              <Badge key={condition} variant="outline" className="font-mono">
                                {condition}
                              </Badge>
                            ))
                          )}
                        </div>
                      </TableCell>
                      <TableCell dir="ltr">
                        <span className="font-medium">{ruleTarget(rule)}</span>
                        {rule.balancerTag ? <span className="ms-1 text-xs text-muted-foreground">({t("core.balancer")})</span> : null}
                      </TableCell>
                      <TableCell>
                        <RowActions
                          index={index}
                          count={rules.length}
                          onMove={(by) => change((draft) => move(listAt(objectAt(draft, "routing"), "rules"), index, by))}
                          onEdit={() => {
                            setRuleAt(index);
                            setRuleOpen(true);
                          }}
                          onDelete={() => change((draft) => void listAt(objectAt(draft, "routing"), "rules").splice(index, 1))}
                          deleteTitle={t("core.deleteRule")}
                          deleteBody={t("core.deleteRuleBody", { number: index + 1 })}
                        />
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle>{t("core.balancers")}</CardTitle>
            <CardDescription className="mt-1 max-w-2xl">{t("core.balancersBody")}</CardDescription>
          </div>
          <Button
            variant="outline"
            onClick={() => {
              setBalancerAt(null);
              setBalancerOpen(true);
            }}
          >
            <PlusIcon /> {t("core.addBalancer")}
          </Button>
        </CardHeader>
        {balancers.length > 0 && (
          <CardContent className="p-0">
            <Table>
              <TableBody>
                {balancers.map((balancer, index) => (
                  <TableRow key={String(balancer.tag)}>
                    <TableCell className="font-medium" dir="ltr">
                      {String(balancer.tag)}
                    </TableCell>
                    <TableCell className="text-muted-foreground" dir="ltr">
                      {t(`core.strategy_${fromBalancer(balancer).strategy}`, { defaultValue: fromBalancer(balancer).strategy })} · {fromBalancer(balancer).selector.split("\n").join(", ")}
                    </TableCell>
                    <TableCell>
                      <RowActions
                        index={index}
                        count={balancers.length}
                        onEdit={() => {
                          setBalancerAt(index);
                          setBalancerOpen(true);
                        }}
                        onDelete={() => change((draft) => void listAt(objectAt(draft, "routing"), "balancers").splice(index, 1))}
                        deleteTitle={t("core.deleteBalancer")}
                        deleteBody={t("core.deleteBalancerBody", { tag: String(balancer.tag) })}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        )}
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("core.observatory")}</CardTitle>
          <CardDescription>{t("core.observatoryBody")}</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          <SwitchField label={t("core.observatoryOn")} checked={observatory.enabled} onCheckedChange={(enabled) => setObservatory({ enabled })} />
          {observatory.enabled && (
            <div className="grid gap-4 sm:grid-cols-2">
              <TextareaField
                label={t("core.observatorySubjects")}
                value={observatory.subjectSelector}
                onChange={(e) => setObservatory({ subjectSelector: e.target.value })}
                dir="ltr"
                spellCheck={false}
                className="min-h-20 font-mono text-xs"
                hint={t("core.observatorySubjectsHint")}
              />
              <div className="grid content-start gap-4">
                <Field label={t("core.observatoryUrl")} value={observatory.probeUrl} onChange={(e) => setObservatory({ probeUrl: e.target.value })} dir="ltr" />
                <Field label={t("core.observatoryInterval")} value={observatory.probeInterval} onChange={(e) => setObservatory({ probeInterval: e.target.value })} dir="ltr" hint="60s, 5m, 1h" />
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Geodata {...props} />

      <RuleDialog
        open={ruleOpen}
        onOpenChange={setRuleOpen}
        rule={ruleAt === null ? null : rules[ruleAt]}
        settings={settings}
        inbounds={core.inbounds}
        onSave={(rule) =>
          change((draft) => {
            const list = listAt(objectAt(draft, "routing"), "rules");
            if (ruleAt === null) list.push(rule);
            else list[ruleAt] = rule;
          })
        }
      />
      <BalancerDialog
        open={balancerOpen}
        onOpenChange={setBalancerOpen}
        balancer={balancerAt === null ? null : balancers[balancerAt]}
        settings={settings}
        onSave={(balancer) =>
          change((draft) => {
            const list = listAt(objectAt(draft, "routing"), "balancers");
            if (balancerAt === null) list.push(balancer);
            else list[balancerAt] = balancer;
          })
        }
      />
    </div>
  );
}
