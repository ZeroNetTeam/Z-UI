import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { GroupPicker } from "@/components/pickers";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Segmented } from "@/components/ui/segmented";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import { emptyAdmin, fromAdmin, toAdminPayload, type AdminForm } from "@/lib/admin-form";
import { api, post, put, type AdminRow, type RoleList } from "@/lib/api";
import { useMe } from "@/lib/permissions";

export function useRoles() {
  return useQuery({ queryKey: ["roles"], queryFn: () => api<RoleList>("roles") });
}

/** Make an admin, or change one: role, quotas, the groups they may give. */
export function AdminDialog({
  open,
  onOpenChange,
  admin,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  admin?: AdminRow;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const me = useMe();
  const roles = useRoles();
  const [form, setForm] = useState<AdminForm>(emptyAdmin());
  const set = (patch: Partial<AdminForm>) => setForm((current) => ({ ...current, ...patch }));
  // Changing oneself is how to lock oneself out; the server refuses it too.
  const self = admin !== undefined && admin.id === me?.id;

  useEffect(() => {
    if (!open) return;
    setForm(admin ? fromAdmin(admin) : emptyAdmin(roles.data?.roles[0]?.id));
    save.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, admin]);

  // A new admin starts with the first role, once the roles are known.
  const firstRole = roles.data?.roles[0]?.id;
  useEffect(() => {
    if (open && !admin && firstRole !== undefined) {
      setForm((current) => (current.roleId === "" && !current.isSudo ? { ...current, roleId: String(firstRole) } : current));
    }
  }, [open, admin, firstRole]);

  const save = useMutation({ meta: { success: "toast.saved" },
    mutationFn: () => {
      const body = toAdminPayload(form, !admin);
      return admin ? put<AdminRow>(`admins/${admin.id}`, body) : post<AdminRow>("admins", body);
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["admins"] });
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  const roleOptions = (roles.data?.roles ?? []).map((role) => ({
    value: String(role.id),
    label: role.own_users_only ? `${role.name} · ${t("access.ownUsersTag")}` : role.name,
  }));

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={admin ? t("access.editAdmin") : t("access.newAdmin")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label={t("access.username")}
              value={form.username}
              onChange={(e) => set({ username: e.target.value })}
              dir="ltr"
              required
              readOnly={!!admin}
              disabled={!!admin}
              autoComplete="off"
              maxLength={64}
            />
            <Field
              label={admin ? t("access.newPassword") : t("access.password")}
              hint={admin ? t("access.newPasswordHint") : t("access.passwordHint")}
              type="password"
              value={form.password}
              onChange={(e) => set({ password: e.target.value })}
              dir="ltr"
              required={!admin}
              autoComplete="new-password"
            />
          </div>

          <div className="grid gap-2">
            <span className="text-sm font-medium">{t("access.kind")}</span>
            <Segmented
              label={t("access.kind")}
              value={form.isSudo ? "sudo" : "role"}
              onChange={(kind) => set({ isSudo: kind === "sudo" })}
              options={[
                { value: "role", label: t("access.kindRole") },
                { value: "sudo", label: t("access.kindSudo") },
              ]}
              className={self ? "pointer-events-none opacity-60" : undefined}
            />
            <p className="text-xs text-muted-foreground">{form.isSudo ? t("access.sudoHint") : t("access.roleHint")}</p>
          </div>

          {!form.isSudo && (
            <>
              <SelectField
                label={t("access.role")}
                value={form.roleId}
                onChange={(e) => set({ roleId: e.target.value })}
                options={[{ value: "", label: t("access.pickRole") }, ...roleOptions]}
                required
                disabled={self}
              />
              <div className="grid gap-4 sm:grid-cols-2">
                <Field
                  label={t("access.userLimit")}
                  hint={t("access.userLimitHint")}
                  type="number"
                  min={0}
                  step={1}
                  inputMode="numeric"
                  value={form.userLimit}
                  onChange={(e) => set({ userLimit: e.target.value })}
                  dir="ltr"
                />
                <Field
                  label={t("access.dataQuota")}
                  hint={t("access.dataQuotaHint")}
                  type="number"
                  min={0}
                  step="any"
                  inputMode="decimal"
                  value={form.dataQuotaGb}
                  onChange={(e) => set({ dataQuotaGb: e.target.value })}
                  dir="ltr"
                />
              </div>
              <GroupPicker value={form.groupIds} onChange={(groupIds) => set({ groupIds })} hint={t("access.groupsHint")} />
            </>
          )}

          <SwitchField
            label={t("access.enabled")}
            hint={self ? t("access.selfHint") : t("access.enabledHint")}
            checked={form.enabled}
            onCheckedChange={(enabled) => set({ enabled })}
            disabled={self}
          />
          <Field label={t("access.note")} value={form.note} onChange={(e) => set({ note: e.target.value })} maxLength={512} />

          <FormError error={save.error ?? roles.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {admin ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
