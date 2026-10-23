// Template cards adapted from PasarGuard's dashboard (features/templates/components/user-template.tsx), AGPL-3.0.
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { CalendarIcon, DatabaseIcon, GaugeIcon, LayoutTemplateIcon, PencilIcon, PlusIcon, RotateCcwIcon, Trash2Icon, UsersIcon } from "lucide-react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { Can } from "@/components/can";
import { PageHeader } from "@/components/layout/layout";
import { GroupPicker, useGroups } from "@/components/pickers";
import { useTemplates } from "@/components/user-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { del, post, put, type ResetStrategy, type UserTemplate } from "@/lib/api";
import { useOpenFromLink } from "@/lib/open-from-link";
import { bytesToGb, bytesToMbit, gbToBytes, mbitToBytes, resetStrategies } from "@/lib/users";
import { formatBytes } from "@/lib/utils";

interface TemplateForm {
  name: string;
  dataLimitGb: string;
  expireDays: string;
  onHold: boolean;
  resetStrategy: ResetStrategy;
  ipLimit: string;
  hwidLimit: string;
  speedUpMbit: string;
  speedDownMbit: string;
  groupIds: number[];
}

function fromTemplate(template?: UserTemplate): TemplateForm {
  return {
    name: template?.name ?? "",
    dataLimitGb: bytesToGb(template?.data_limit ?? 0),
    expireDays: template?.expire_days ? String(template.expire_days) : "",
    onHold: template?.on_hold ?? false,
    resetStrategy: template?.reset_strategy ?? "no_reset",
    ipLimit: template?.ip_limit ? String(template.ip_limit) : "",
    hwidLimit: template?.hwid_limit ? String(template.hwid_limit) : "",
    speedUpMbit: bytesToMbit(template?.speed_up ?? 0),
    speedDownMbit: bytesToMbit(template?.speed_down ?? 0),
    groupIds: template?.group_ids ?? [],
  };
}

const whole = (value: string) => Math.max(0, Math.floor(Number(value)) || 0);

function TemplateDialog({ open, onOpenChange, template }: { open: boolean; onOpenChange: (open: boolean) => void; template?: UserTemplate }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [form, setForm] = useState<TemplateForm>(fromTemplate());
  const set = (patch: Partial<TemplateForm>) => setForm((current) => ({ ...current, ...patch }));
  useEffect(() => {
    if (!open) return;
    setForm(fromTemplate(template));
    save.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, template]);
  const save = useMutation({
    mutationFn: () => {
      const body = {
        name: form.name.trim(),
        data_limit: gbToBytes(form.dataLimitGb),
        expire_days: whole(form.expireDays),
        on_hold: form.onHold,
        reset_strategy: form.resetStrategy,
        ip_limit: whole(form.ipLimit),
        hwid_limit: whole(form.hwidLimit),
        speed_up: mbitToBytes(form.speedUpMbit),
        speed_down: mbitToBytes(form.speedDownMbit),
        group_ids: form.groupIds,
      };
      return template ? put(`templates/${template.id}`, body) : post("templates", body);
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["templates"] });
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={template ? t("templates.editTitle") : t("templates.newTitle")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("templates.name")} value={form.name} onChange={(e) => set({ name: e.target.value })} required maxLength={64} />
          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label={t("users.dataLimit")}
              type="number"
              min={0}
              step="any"
              value={form.dataLimitGb}
              onChange={(e) => set({ dataLimitGb: e.target.value })}
              placeholder="∞"
              dir="ltr"
            />
            <Field
              label={t("templates.days")}
              type="number"
              min={0}
              value={form.expireDays}
              onChange={(e) => set({ expireDays: e.target.value })}
              placeholder="∞"
              dir="ltr"
            />
          </div>
          <SwitchField
            label={t("templates.onHold")}
            hint={t("templates.onHoldHint")}
            checked={form.onHold}
            onCheckedChange={(onHold) => set({ onHold })}
          />
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("users.resetStrategy")}
              value={form.resetStrategy}
              onChange={(e) => set({ resetStrategy: e.target.value as ResetStrategy })}
              options={resetStrategies.map((value) => ({ value, label: t(`users.reset.${value}`) }))}
            />
            <Field label={t("users.ipLimit")} type="number" min={0} value={form.ipLimit} onChange={(e) => set({ ipLimit: e.target.value })} placeholder="∞" dir="ltr" />
            <Field label={t("users.hwidLimit")} type="number" min={0} value={form.hwidLimit} onChange={(e) => set({ hwidLimit: e.target.value })} placeholder="∞" dir="ltr" />
            <Field label={t("users.speedDown")} type="number" min={0} step="any" value={form.speedDownMbit} onChange={(e) => set({ speedDownMbit: e.target.value })} placeholder="∞" dir="ltr" />
            <Field label={t("users.speedUp")} type="number" min={0} step="any" value={form.speedUpMbit} onChange={(e) => set({ speedUpMbit: e.target.value })} placeholder="∞" dir="ltr" />
          </div>
          <GroupPicker value={form.groupIds} onChange={(groupIds) => set({ groupIds })} hint={t("templates.groupsHint")} />
          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {template ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Line({ icon: Icon, children }: { icon: typeof DatabaseIcon; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 text-sm">
      <Icon className="size-4 shrink-0 text-primary/80" />
      <span className="min-w-0 truncate">{children}</span>
    </div>
  );
}

export function TemplatesPage() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const templates = useTemplates();
  const groups = useGroups();
  const [editing, setEditing] = useState<UserTemplate | undefined>();
  const [open, setOpen] = useState(false);
  useOpenFromLink("template", () => setOpen(true));
  const groupName = (id: number) => groups.data?.find((group) => group.id === id)?.name ?? `#${id}`;
  const speed = (bytes: number) => (bytes ? `${new Intl.NumberFormat(i18n.language).format(Number(bytesToMbit(bytes)))} Mbit/s` : "∞");

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("templates.title")} subtitle={t("templates.subtitle")} />
        <Can perm="templates.write">
<Button
          onClick={() => {
            setEditing(undefined);
            setOpen(true);
          }}
        >
          <PlusIcon /> {t("templates.add")}
        </Button>
</Can>
      </div>
      <div className="mb-4">
        <FormError error={templates.error} />
      </div>
      {templates.isPending ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <Skeleton className="h-44" />
          <Skeleton className="h-44" />
        </div>
      ) : templates.data?.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
            <LayoutTemplateIcon className="size-8 text-primary/60" />
            {t("templates.empty")}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {templates.data?.map((template) => (
            <Card key={template.id}>
              <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
                <CardTitle className="truncate">{template.name}</CardTitle>
                <Can perm="templates.write">
<div className="flex shrink-0 gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-8"
                    aria-label={t("common.edit")}
                    onClick={() => {
                      setEditing(template);
                      setOpen(true);
                    }}
                  >
                    <PencilIcon />
                  </Button>
                  <Confirm
                    title={t("templates.deleteTitle")}
                    body={t("templates.deleteBody", { name: template.name })}
                    action={t("common.delete")}
                    onConfirm={async () => {
                      await del(`templates/${template.id}`);
                      await client.invalidateQueries({ queryKey: ["templates"] });
                    }}
                    trigger={(confirm) => (
                      <Button variant="ghost" size="icon" className="size-8" aria-label={t("common.delete")} onClick={confirm}>
                        <Trash2Icon className="text-destructive" />
                      </Button>
                    )}
                  />
                </div>
</Can>
              </CardHeader>
              <CardContent className="grid gap-2.5">
                <Line icon={DatabaseIcon}>
                  <span dir="ltr">{template.data_limit ? formatBytes(template.data_limit, i18n.language) : "∞"}</span>
                </Line>
                <Line icon={CalendarIcon}>
                  {template.expire_days
                    ? t(template.on_hold ? "templates.daysOnHold" : "templates.daysFromCreation", { count: template.expire_days })
                    : t("users.neverExpires")}
                </Line>
                <Line icon={RotateCcwIcon}>{t(`users.reset.${template.reset_strategy}`)}</Line>
                <Line icon={GaugeIcon}>
                  <span dir="ltr">
                    ↓ {speed(template.speed_down)} · ↑ {speed(template.speed_up)}
                  </span>
                </Line>
                <Line icon={UsersIcon}>
                  {template.group_ids.length ? template.group_ids.map(groupName).join("، ") : t("templates.keepGroups")}
                </Line>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
      <TemplateDialog open={open} onOpenChange={setOpen} template={editing} />
    </>
  );
}
