import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { PermissionMatrix } from "@/components/permission-matrix";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SwitchField } from "@/components/ui/switch";
import { post, put, type Role, type RolePayload } from "@/lib/api";
import { minimal } from "@/lib/permissions";

/** Make a role, or change one: what it may do in each part of the panel. */
export function RoleDialog({
  open,
  onOpenChange,
  role,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  role?: Role;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [permissions, setPermissions] = useState<string[]>([]);
  const [own, setOwn] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(role?.name ?? "");
    setPermissions(role?.permissions ?? []);
    setOwn(role?.own_users_only ?? false);
    save.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, role]);

  const save = useMutation({
    mutationFn: () => {
      const body: RolePayload = { name: name.trim(), permissions: minimal(permissions), own_users_only: own };
      return role ? put(`roles/${role.id}`, body) : post("roles", body);
    },
    onSuccess: async () => {
      // The admins' list names each one's role.
      await client.invalidateQueries({ queryKey: ["roles"] });
      await client.invalidateQueries({ queryKey: ["admins"] });
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={role ? t("access.editRole") : t("access.newRole")} className="max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("access.roleName")} value={name} onChange={(e) => setName(e.target.value)} required maxLength={64} />
          <SwitchField label={t("access.ownUsers")} hint={t("access.ownUsersHint")} checked={own} onCheckedChange={setOwn} />
          <div className="grid gap-2">
            <span className="text-sm font-medium">{t("access.permissions")}</span>
            <PermissionMatrix value={permissions} onChange={setPermissions} />
            <p className="text-xs text-muted-foreground">{t("access.permissionsHint")}</p>
          </div>
          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {role ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
