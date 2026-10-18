import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { GroupPicker, useGroups } from "@/components/pickers";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { TextareaField } from "@/components/ui/textarea";
import { api, post, put, type Connected, type User, type UserTemplate } from "@/lib/api";
import { emptyForm, expiryIn, fromUser, toPayload, withTemplate, type UserForm } from "@/lib/user-form";
import { daysUntil, formatDate, resetStrategies } from "@/lib/users";
import { cn } from "@/lib/utils";

export function useTemplates() {
  return useQuery({ queryKey: ["templates"], queryFn: () => api<UserTemplate[]>("templates") });
}

function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <div className="grid gap-4 rounded-lg border p-4">
      {title && <div className="text-sm font-medium">{title}</div>}
      {children}
    </div>
  );
}

/** A value to read and copy (a credential). */
function Copyable({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center justify-between gap-3 text-xs">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="flex min-w-0 items-center gap-1">
        <code dir="ltr" className="truncate font-mono">
          {value}
        </code>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label={label}
          onClick={() => {
            void navigator.clipboard?.writeText(value).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            });
          }}
        >
          {copied ? <CheckIcon className="text-success" /> : <CopyIcon />}
        </Button>
      </span>
    </div>
  );
}

const presets = [30, 60, 90, 180, 365];

/** Make one user or many alike, or change one. */
export function UserDialog({
  open,
  onOpenChange,
  user,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  user?: User;
}) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const groups = useGroups();
  const templates = useTemplates();
  const [form, setForm] = useState<UserForm>(emptyForm());
  const [expireDays, setExpireDays] = useState("");
  const [many, setMany] = useState(false);
  const [prefix, setPrefix] = useState("user");
  const [start, setStart] = useState("1");
  const [count, setCount] = useState("10");

  const connected = useQuery({
    queryKey: ["users", user?.id, "online"],
    queryFn: () => api<Connected>(`users/${user!.id}/online`),
    enabled: open && !!user?.online,
  });

  const defaultGroups = () => (groups.data ?? []).filter((g) => g.name === "default").map((g) => g.id);

  useEffect(() => {
    if (!open) return;
    const initial = user ? fromUser(user) : emptyForm(defaultGroups());
    setForm(initial);
    setExpireDays(initial.expireAt && daysUntil(initial.expireAt) > 0 ? String(daysUntil(initial.expireAt)) : "");
    setMany(false);
    save.reset();
    // The groups arrive once; a dialog opened before keeps what it had.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, user]);

  const set = (patch: Partial<UserForm>) => setForm((current) => ({ ...current, ...patch }));
  const setDays = (value: string) => {
    setExpireDays(value);
    set({ expireAt: expiryIn(value) });
  };

  const save = useMutation({
    mutationFn: async () => {
      const payload = toPayload(form);
      if (user) return put<User>(`users/${user.id}`, payload);
      if (many) {
        const { username: _, ...rest } = payload;
        return post<{ changed: number }>("users/bulk-create", {
          ...rest,
          prefix: prefix.trim(),
          start: Math.max(0, Math.floor(Number(start)) || 0),
          count: Math.floor(Number(count)) || 0,
        });
      }
      return post<User>("users", payload);
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["users"] });
      await client.invalidateQueries({ queryKey: ["groups"] });
      onOpenChange(false);
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  const onHold = form.status === "on_hold";
  const past = form.expireAt !== null && daysUntil(form.expireAt) <= 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={user ? t("users.editTitle", { name: user.username }) : t("users.newTitle")} className="max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          {!user && (
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="flex rounded-lg border p-1 text-sm" role="radiogroup">
                {[false, true].map((value) => (
                  <button
                    key={String(value)}
                    type="button"
                    role="radio"
                    aria-checked={many === value}
                    onClick={() => setMany(value)}
                    className={cn(
                      "flex-1 rounded-md px-3 py-1.5 transition-colors",
                      many === value ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent",
                    )}
                  >
                    {value ? t("users.many") : t("users.one")}
                  </button>
                ))}
              </div>
              {(templates.data?.length ?? 0) > 0 && (
                <SelectField
                  label={t("users.fromTemplate")}
                  value=""
                  onChange={(e) => {
                    const template = templates.data?.find((item) => item.id === Number(e.target.value));
                    if (!template) return;
                    const next = withTemplate(form, template);
                    setForm(next);
                    setExpireDays(next.expireAt ? String(template.expire_days) : "");
                  }}
                  options={[
                    { value: "", label: t("users.pickTemplate") },
                    ...(templates.data ?? []).map((item) => ({ value: String(item.id), label: item.name })),
                  ]}
                />
              )}
            </div>
          )}

          {many && !user ? (
            <div className="grid gap-4 sm:grid-cols-3">
              <Field label={t("users.prefix")} value={prefix} onChange={(e) => setPrefix(e.target.value)} dir="ltr" required />
              <Field label={t("users.start")} type="number" min={0} value={start} onChange={(e) => setStart(e.target.value)} dir="ltr" />
              <Field
                label={t("users.count")}
                type="number"
                min={1}
                max={1000}
                value={count}
                onChange={(e) => setCount(e.target.value)}
                dir="ltr"
                hint={t("users.namesLike", { first: `${prefix.trim()}${start}`, last: `${prefix.trim()}${Number(start) + Math.max(1, Number(count)) - 1}` })}
              />
            </div>
          ) : (
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("users.username")}
                value={form.username}
                onChange={(e) => set({ username: e.target.value })}
                dir="ltr"
                autoComplete="off"
                spellCheck={false}
                required
                minLength={3}
                maxLength={64}
                pattern="[A-Za-z0-9._@\-]+"
                hint={t("users.usernameHint")}
              />
              <SelectField
                label={t("users.statusLabel")}
                value={form.status}
                onChange={(e) => set({ status: e.target.value as UserForm["status"] })}
                options={(["active", "on_hold", "disabled"] as const).map((value) => ({ value, label: t(`users.status.${value}`) }))}
                hint={onHold ? t("users.onHoldHint") : undefined}
              />
            </div>
          )}

          <Section title={t("users.limits")}>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label={t("users.dataLimit")}
                type="number"
                min={0}
                step="any"
                inputMode="decimal"
                value={form.dataLimitGb}
                onChange={(e) => set({ dataLimitGb: e.target.value })}
                placeholder="∞"
                dir="ltr"
                hint={t("users.dataLimitHint")}
              />
              <SelectField
                label={t("users.resetStrategy")}
                value={form.resetStrategy}
                onChange={(e) => set({ resetStrategy: e.target.value as UserForm["resetStrategy"] })}
                options={resetStrategies.map((value) => ({ value, label: t(`users.reset.${value}`) }))}
              />
            </div>

            {onHold ? (
              <div className="grid gap-4 sm:grid-cols-2">
                <Field
                  label={t("users.onHoldDays")}
                  type="number"
                  min={1}
                  value={form.onHoldDays}
                  onChange={(e) => set({ onHoldDays: e.target.value })}
                  dir="ltr"
                  required
                  hint={t("users.onHoldDaysHint")}
                />
                <Field
                  label={t("users.onHoldTimeout")}
                  type="number"
                  min={0}
                  value={form.onHoldTimeoutDays}
                  onChange={(e) => set({ onHoldTimeoutDays: e.target.value })}
                  placeholder={t("common.optional")}
                  dir="ltr"
                  hint={t("users.onHoldTimeoutHint")}
                />
              </div>
            ) : (
              <div className="grid gap-2">
                <Field
                  label={t("users.expireDays")}
                  type="number"
                  min={0}
                  value={expireDays}
                  onChange={(e) => setDays(e.target.value)}
                  placeholder="∞"
                  dir="ltr"
                />
                <div className="flex flex-wrap gap-1.5">
                  <Button type="button" variant={form.expireAt === null ? "secondary" : "outline"} size="sm" onClick={() => setDays("")}>
                    {t("users.noExpiry")}
                  </Button>
                  {presets.map((days) => (
                    <Button
                      key={days}
                      type="button"
                      variant={expireDays === String(days) ? "secondary" : "outline"}
                      size="sm"
                      onClick={() => setDays(String(days))}
                    >
                      {t("common.days", { count: days })}
                    </Button>
                  ))}
                </div>
                <p className={cn("text-xs", past ? "text-destructive" : "text-muted-foreground")}>
                  {form.expireAt === null
                    ? t("users.neverExpires")
                    : past
                      ? t("users.expiredOn", { date: formatDate(form.expireAt, i18n.language) })
                      : t("users.expiresOn", { date: formatDate(form.expireAt, i18n.language, true) })}
                </p>
              </div>
            )}
          </Section>

          <Section>
            <SwitchField
              label={t("users.nextPlan")}
              hint={t("users.nextPlanHint")}
              checked={form.nextPlan}
              onCheckedChange={(nextPlan) => set({ nextPlan })}
            />
            {form.nextPlan && (
              <>
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field
                    label={t("users.dataLimit")}
                    type="number"
                    min={0}
                    step="any"
                    value={form.nextDataGb}
                    onChange={(e) => set({ nextDataGb: e.target.value })}
                    placeholder="∞"
                    dir="ltr"
                  />
                  <Field
                    label={t("users.nextDays")}
                    type="number"
                    min={0}
                    value={form.nextDays}
                    onChange={(e) => set({ nextDays: e.target.value })}
                    placeholder="∞"
                    dir="ltr"
                  />
                </div>
                <SwitchField
                  label={t("users.carryOver")}
                  hint={t("users.carryOverHint")}
                  checked={form.nextCarryOver}
                  onCheckedChange={(nextCarryOver) => set({ nextCarryOver })}
                />
              </>
            )}
          </Section>

          <GroupPicker value={form.groupIds} onChange={(groupIds) => set({ groupIds })} hint={t("users.groupsHint")} />

          <Section title={t("common.advanced")}>
            <div className="grid gap-4 sm:grid-cols-3">
              <Field
                label={t("users.ipLimit")}
                type="number"
                min={0}
                value={form.ipLimit}
                onChange={(e) => set({ ipLimit: e.target.value })}
                placeholder="∞"
                dir="ltr"
                hint={t("users.ipLimitHint")}
              />
              <Field
                label={t("users.speedDown")}
                type="number"
                min={0}
                step="any"
                value={form.speedDownMbit}
                onChange={(e) => set({ speedDownMbit: e.target.value })}
                placeholder="∞"
                dir="ltr"
              />
              <Field
                label={t("users.speedUp")}
                type="number"
                min={0}
                step="any"
                value={form.speedUpMbit}
                onChange={(e) => set({ speedUpMbit: e.target.value })}
                placeholder="∞"
                dir="ltr"
              />
            </div>
            <TextareaField label={t("users.note")} value={form.note} onChange={(e) => set({ note: e.target.value })} className="min-h-16" />
          </Section>

          {user?.online && (
            <Section title={t("users.connected", { count: connected.data?.sessions ?? 0 })}>
              {connected.data?.addresses.length ? (
                <div className="flex flex-wrap gap-1.5">
                  {connected.data.addresses.map((address) => (
                    <code key={address} dir="ltr" className="rounded-md bg-muted px-2 py-1 font-mono text-xs">
                      {address}
                    </code>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">{connected.isPending ? t("common.loading") : t("users.noAddresses")}</p>
              )}
            </Section>
          )}

          {user && (
            <Section title={t("users.credentials")}>
              <div className="grid gap-1">
                <Copyable label="UUID" value={user.uuid} />
                <Copyable label={t("users.password")} value={user.password} />
              </div>
            </Section>
          )}

          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {user ? t("common.save") : many ? t("users.createMany", { count: Number(count) || 0 }) : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
