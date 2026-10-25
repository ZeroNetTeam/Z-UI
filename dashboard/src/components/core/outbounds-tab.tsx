import { useMutation } from "@tanstack/react-query";
import { CheckCircle2Icon, CloudIcon, PlusIcon, StarIcon } from "lucide-react";
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
import { useOpenFromLink } from "@/lib/open-from-link";
import {
  canFragment,
  domainStrategies,
  emptyOutbound,
  fromOutbound,
  outboundKind,
  outboundKinds,
  outboundSummary,
  toOutbound,
  usedBy,
  type Json,
  type OutboundForm,
} from "@/lib/core-form";
import { listAt, RowActions, type TabProps } from "./shared";

function OutboundDialog({
  open,
  onOpenChange,
  index,
  settings,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Which outbound is edited; none for a new one. */
  index: number | null;
  settings: Json;
  onSave: (outbound: Json) => void;
}) {
  const { t } = useTranslation();
  const outbounds = (Array.isArray(settings.outbounds) ? settings.outbounds : []) as Json[];
  const previous = index === null ? undefined : outbounds[index];
  const [form, setForm] = useState<OutboundForm>(emptyOutbound());
  const [problem, setProblem] = useState<string | null>(null);
  const [checked, setChecked] = useState<string | null>(null);
  const set = (patch: Partial<OutboundForm>) => setForm((current) => ({ ...current, ...patch }));

  useEffect(() => {
    if (!open) return;
    setForm(previous ? fromOutbound(previous) : emptyOutbound());
    setProblem(null);
    setChecked(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, index]);

  const check = useMutation({ meta: { quiet: true },
    mutationFn: () => post<{ remark: string; scheme: string }>("core/check-link", { link: form.link }),
    onSuccess: (info) => setChecked(`${info.scheme}${info.remark ? ` · ${info.remark}` : ""}`),
    onError: () => setChecked(null),
  });
  const warp = useMutation({ meta: { quiet: true },
    mutationFn: () => post<{ link: string }>("core/warp"),
    onSuccess: (account) => {
      setChecked(null);
      setForm((current) => ({ ...current, link: account.link, tag: current.tag || "warp" }));
    },
  });

  const otherTags = outbounds.map((o) => String(o.tag ?? "")).filter((tag) => tag && tag !== previous?.tag);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const tag = form.tag.trim();
    if (!tag) return setProblem(t("core.tagRequired"));
    if (otherTags.includes(tag)) return setProblem(t("core.tagTaken"));
    try {
      onSave(toOutbound(form, previous));
      onOpenChange(false);
    } catch (error) {
      setProblem(t("core.jsonInvalid", { message: (error as Error).message }));
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={previous ? t("core.editOutbound") : t("core.newOutbound")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("core.kind")}
              value={form.kind}
              disabled={index !== null}
              onChange={(e) => set({ kind: e.target.value as OutboundForm["kind"] })}
              options={outboundKinds.map((kind) => ({ value: kind, label: t(`core.kind_${kind}`) }))}
            />
            <Field label={t("core.tag")} value={form.tag} onChange={(e) => set({ tag: e.target.value })} dir="ltr" required hint={t("core.tagHint")} />
          </div>

          {form.kind === "link" && (
            <>
              <TextareaField
                label={t("core.link")}
                value={form.link}
                onChange={(e) => {
                  set({ link: e.target.value });
                  setChecked(null);
                }}
                dir="ltr"
                spellCheck={false}
                className="min-h-20 font-mono text-xs"
                placeholder="vless://…  vmess://…  trojan://…  ss://…  hysteria2://…  warp://…"
                hint={t("core.linkHint")}
                required
              />
              <div className="flex flex-wrap items-center gap-2">
                <Button type="button" variant="outline" size="sm" disabled={!form.link.trim() || check.isPending} onClick={() => check.mutate()}>
                  {t("common.check")}
                </Button>
                <Button type="button" variant="outline" size="sm" disabled={warp.isPending} onClick={() => warp.mutate()}>
                  <CloudIcon /> {warp.isPending ? t("core.warpWorking") : t("core.warpCreate")}
                </Button>
                {checked && (
                  <span className="flex items-center gap-1.5 text-sm text-success" dir="auto">
                    <CheckCircle2Icon className="size-4" /> {checked}
                  </span>
                )}
              </div>
              <p className="-mt-2 text-xs text-muted-foreground">{t("core.warpHint")}</p>
              <FormError error={check.error ?? warp.error} />
              <SelectField
                label={t("core.dialThrough")}
                value={form.dialThrough}
                onChange={(e) => set({ dialThrough: e.target.value })}
                options={[{ value: "", label: t("core.dialDirect") }, ...otherTags.map((tag) => ({ value: tag, label: tag }))]}
                hint={t("core.dialThroughHint")}
              />
              <div className="grid gap-3">
                <SwitchField
                  label={t("core.fragment")}
                  hint={t("core.fragmentHint")}
                  checked={form.fragment}
                  onCheckedChange={(fragment) => set({ fragment })}
                />
                <SwitchField
                  label={t("core.allowInsecure")}
                  hint={t("core.allowInsecureHint")}
                  checked={form.allowInsecure}
                  onCheckedChange={(allowInsecure) => set({ allowInsecure })}
                />
              </div>
            </>
          )}

          {form.kind === "freedom" && (
            <SelectField
              label={t("core.domainStrategy")}
              value={form.domainStrategy}
              onChange={(e) => set({ domainStrategy: e.target.value })}
              options={domainStrategies.map((value) => ({ value, label: value }))}
              hint={t("core.domainStrategyHint")}
            />
          )}

          {form.kind === "blackhole" && <p className="text-sm text-muted-foreground">{t("core.blackholeHint")}</p>}
          {form.kind === "tor" && <p className="text-sm text-muted-foreground">{t("core.torHint")}</p>}

          {form.kind === "json" && (
            <TextareaField
              label={t("core.json")}
              value={form.json}
              onChange={(e) => set({ json: e.target.value })}
              dir="ltr"
              spellCheck={false}
              className="min-h-56 font-mono text-xs"
              hint={t("core.jsonOutboundHint")}
            />
          )}

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

export function OutboundsTab({ settings, change }: TabProps) {
  const { t } = useTranslation();
  const outbounds = (Array.isArray(settings.outbounds) ? settings.outbounds : []) as Json[];
  const [editing, setEditing] = useState<number | null>(null);
  const [open, setOpen] = useState(false);
  useOpenFromLink("outbound", () => setOpen(true));

  return (
    <Card>
      <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
        <div>
          <CardTitle>{t("core.outbounds")}</CardTitle>
          <CardDescription className="mt-1 max-w-2xl">{t("core.outboundsBody")}</CardDescription>
        </div>
        <Button
          onClick={() => {
            setEditing(null);
            setOpen(true);
          }}
        >
          <PlusIcon /> {t("core.addOutbound")}
        </Button>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("core.tag")}</TableHead>
              <TableHead>{t("core.reaches")}</TableHead>
              <TableHead className="text-end">{t("common.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {outbounds.map((outbound, index) => {
              const tag = String(outbound.tag ?? "");
              const uses = usedBy(settings, tag);
              return (
                <TableRow key={`${tag}-${index}`}>
                  <TableCell>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium" dir="ltr">
                        {tag || "—"}
                      </span>
                      {index === 0 && <Badge>{t("core.default")}</Badge>}
                      <Badge variant="outline">{t(`core.kind_${outboundKind(outbound)}`)}</Badge>
                    </div>
                  </TableCell>
                  <TableCell className="max-w-xs truncate text-muted-foreground" dir="ltr">
                    {outboundSummary(outbound)}
                    {canFragment(outbound) && Object.keys((outbound.evasion as Json) ?? {}).length > 0 ? " · fragment" : ""}
                  </TableCell>
                  <TableCell>
                    <RowActions
                      index={index}
                      count={outbounds.length}
                      extra={
                        index > 0 ? (
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8"
                            aria-label={t("core.makeDefault")}
                            title={t("core.makeDefault")}
                            onClick={() =>
                              change((draft) => {
                                const list = listAt(draft, "outbounds");
                                const [item] = list.splice(index, 1);
                                list.unshift(item);
                              })
                            }
                          >
                            <StarIcon />
                          </Button>
                        ) : undefined
                      }
                      onEdit={() => {
                        setEditing(index);
                        setOpen(true);
                      }}
                      onDelete={() => change((draft) => void listAt(draft, "outbounds").splice(index, 1))}
                      deleteTitle={t("core.deleteOutbound")}
                      deleteBody={
                        uses.length > 0
                          ? t("core.deleteOutboundUsed", { tag, uses: uses.join(", ") })
                          : outbounds.length === 1
                            ? t("core.deleteLastOutbound")
                            : t("core.deleteOutboundBody", { tag })
                      }
                    />
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </CardContent>
      <OutboundDialog
        open={open}
        onOpenChange={setOpen}
        index={editing}
        settings={settings}
        onSave={(outbound) =>
          change((draft) => {
            const list = listAt(draft, "outbounds");
            if (editing === null) list.push(outbound);
            else list[editing] = outbound;
          })
        }
      />
    </Card>
  );
}
