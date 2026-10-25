import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  KeyRoundIcon,
  LogOutIcon,
  MoreHorizontalIcon,
  PencilIcon,
  PlusIcon,
  ShieldCheckIcon,
  ShieldOffIcon,
  Trash2Icon,
  UsersIcon,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router";
import { AdminDialog, useRoles } from "@/components/admin-dialog";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { RoleDialog } from "@/components/role-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Segmented } from "@/components/ui/segmented";
import { SelectField } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { quotaPercent } from "@/lib/admin-form";
import { api, del, post, type AdminRow, type Role } from "@/lib/api";
import { useOpenFromLink } from "@/lib/open-from-link";
import { useMe } from "@/lib/permissions";
import { relative } from "@/lib/users";
import { cn, formatBytes } from "@/lib/utils";

type Tab = "admins" | "roles";

export function AdminsPage() {
  const { t } = useTranslation();
  const [params, setParams] = useSearchParams();
  const tab: Tab = params.get("tab") === "roles" ? "roles" : "admins";
  const setTab = (next: Tab) => setParams(next === "roles" ? { tab: "roles" } : {}, { replace: true });
  const [adminDialog, setAdminDialog] = useState<{ open: boolean; admin?: AdminRow }>({ open: false });
  const [roleDialog, setRoleDialog] = useState<{ open: boolean; role?: Role }>({ open: false });
  useOpenFromLink("admin", () => setAdminDialog({ open: true }));

  return (
    <>
      <PageHeader
        title={t("access.title")}
        subtitle={t("access.subtitle")}
        action={
          tab === "admins" ? (
            <Button onClick={() => setAdminDialog({ open: true })}>
              <PlusIcon /> {t("access.addAdmin")}
            </Button>
          ) : (
            <Button onClick={() => setRoleDialog({ open: true })}>
              <PlusIcon /> {t("access.addRole")}
            </Button>
          )
        }
      />
      <Segmented
        label={t("access.title")}
        value={tab}
        onChange={setTab}
        options={[
          { value: "admins", label: t("access.tabAdmins") },
          { value: "roles", label: t("access.tabRoles") },
        ]}
        className="mb-4"
      />
      {tab === "admins" ? (
        <AdminsTable onEdit={(admin) => setAdminDialog({ open: true, admin })} />
      ) : (
        <RolesTable onEdit={(role) => setRoleDialog({ open: true, role })} />
      )}
      <AdminDialog open={adminDialog.open} onOpenChange={(open) => setAdminDialog((d) => ({ ...d, open }))} admin={adminDialog.admin} />
      <RoleDialog open={roleDialog.open} onOpenChange={(open) => setRoleDialog((d) => ({ ...d, open }))} role={roleDialog.role} />
    </>
  );
}

function AdminsTable({ onEdit }: { onEdit: (admin: AdminRow) => void }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const me = useMe();
  const admins = useQuery({ queryKey: ["admins"], queryFn: () => api<AdminRow[]>("admins") });
  const [removing, setRemoving] = useState<AdminRow | null>(null);
  const refresh = () => client.invalidateQueries({ queryKey: ["admins"] });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: number; action: "sign-out" | "2fa" }) =>
      action === "2fa" ? del(`admins/${id}/2fa`) : post(`admins/${id}/sign-out`),
    onSuccess: refresh,
  });
  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);

  return (
    <>
      <div className="mb-4 grid gap-2">
        <FormError error={admins.error} />
        <FormError error={act.error} />
      </div>
      <Card>
        <CardContent className="p-0">
          {admins.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("access.username")}</TableHead>
                  <TableHead>{t("access.role")}</TableHead>
                  <TableHead className="hidden md:table-cell">{t("access.users")}</TableHead>
                  <TableHead className="hidden lg:table-cell">{t("access.dataQuota")}</TableHead>
                  <TableHead className="hidden lg:table-cell">{t("access.lastLogin")}</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {admins.data?.map((admin) => {
                  const percent = quotaPercent(admin.allocated, admin.data_quota);
                  const self = admin.id === me?.id;
                  return (
                    <TableRow key={admin.id}>
                      <TableCell>
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-medium" dir="ltr">
                            {admin.username}
                          </span>
                          {self && <Badge variant="outline">{t("access.you")}</Badge>}
                          {!admin.enabled && <Badge variant="destructive">{t("access.disabled")}</Badge>}
                          {admin.pending && <Badge variant="outline">{t("access.pending")}</Badge>}
                          {admin.totp_enabled && (
                            <Badge variant="success" title={t("access.twoFactor")}>
                              <ShieldCheckIcon /> 2FA
                            </Badge>
                          )}
                        </div>
                        {admin.note && <div className="max-w-56 truncate text-xs text-muted-foreground">{admin.note}</div>}
                      </TableCell>
                      <TableCell>
                        {admin.is_sudo ? <Badge>{t("access.kindSudo")}</Badge> : <Badge variant="outline">{admin.role ?? "—"}</Badge>}
                      </TableCell>
                      <TableCell className="tabular hidden md:table-cell">
                        {number(admin.users)}
                        {admin.user_limit > 0 && <span className="text-muted-foreground"> / {number(admin.user_limit)}</span>}
                      </TableCell>
                      <TableCell className="hidden lg:table-cell">
                        {admin.is_sudo ? (
                          <span className="text-muted-foreground">—</span>
                        ) : admin.data_quota > 0 ? (
                          <div className="grid w-40 gap-1">
                            <div className="tabular text-xs" dir="ltr">
                              {formatBytes(admin.allocated, i18n.language)} / {formatBytes(admin.data_quota, i18n.language)}
                            </div>
                            <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                              <div
                                className={cn("h-full rounded-full", (percent ?? 0) >= 90 ? "bg-warning" : "bg-primary")}
                                style={{ width: `${percent}%` }}
                              />
                            </div>
                          </div>
                        ) : (
                          <span className="text-xs text-muted-foreground">
                            {t("access.noQuota")}
                            {admin.allocated > 0 && ` · ${formatBytes(admin.allocated, i18n.language)}`}
                          </span>
                        )}
                        {!admin.is_sudo && admin.data_quota > 0 && admin.unlimited > 0 && (
                          <div className="text-[11px] text-warning">{t("access.unlimitedUsers", { count: admin.unlimited })}</div>
                        )}
                      </TableCell>
                      <TableCell className="hidden text-xs text-muted-foreground lg:table-cell">
                        {admin.last_login_at ? relative(admin.last_login_at, i18n.language) : t("access.never")}
                      </TableCell>
                      <TableCell>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button variant="ghost" size="icon" aria-label={t("common.actions")}>
                              <MoreHorizontalIcon />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => onEdit(admin)}>
                              <PencilIcon /> {t("common.edit")}
                            </DropdownMenuItem>
                            <DropdownMenuItem onSelect={() => act.mutate({ id: admin.id, action: "sign-out" })}>
                              <LogOutIcon /> {t("access.signOut")}
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              disabled={!admin.totp_enabled || self}
                              onSelect={() => act.mutate({ id: admin.id, action: "2fa" })}
                            >
                              <ShieldOffIcon /> {t("access.reset2fa")}
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              disabled={self}
                              onSelect={() => setRemoving(admin)}
                              className="text-destructive data-[highlighted]:text-destructive"
                            >
                              <Trash2Icon /> {t("common.delete")}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
      <RemoveAdminDialog admin={removing} onOpenChange={(open) => !open && setRemoving(null)} />
    </>
  );
}

/** Delete an admin; their users go to the one picked. */
function RemoveAdminDialog({ admin, onOpenChange }: { admin: AdminRow | null; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const me = useMe();
  const admins = useQuery({ queryKey: ["admins"], queryFn: () => api<AdminRow[]>("admins") });
  const [heir, setHeir] = useState("");
  const remove = useMutation({
    mutationFn: () => del(`admins/${admin?.id}?transfer_to=${heir || me?.id}`),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["admins"] });
      await client.invalidateQueries({ queryKey: ["users"] });
      onOpenChange(false);
    },
  });
  const others = (admins.data ?? []).filter((other) => other.id !== admin?.id && other.enabled);
  return (
    <Dialog
      open={admin !== null}
      onOpenChange={(open) => {
        if (open) remove.reset();
        onOpenChange(open);
      }}
    >
      <DialogContent title={t("access.deleteTitle", { name: admin?.username ?? "" })} className="max-w-md">
        <div className="grid gap-4">
          <p className="text-sm text-muted-foreground">{t("access.deleteBody", { count: admin?.users ?? 0 })}</p>
          {(admin?.users ?? 0) > 0 && (
            <SelectField
              label={t("access.heir")}
              value={heir || String(me?.id ?? "")}
              onChange={(e) => setHeir(e.target.value)}
              options={others.map((other) => ({ value: String(other.id), label: other.username }))}
            />
          )}
          <FormError error={remove.error} />
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="destructive" disabled={remove.isPending} onClick={() => remove.mutate()}>
              {t("common.delete")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function RolesTable({ onEdit }: { onEdit: (role: Role) => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const roles = useRoles();
  return (
    <>
      <div className="mb-4">
        <FormError error={roles.error} />
      </div>
      <Card>
        <CardContent className="p-0">
          {roles.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : roles.data?.roles.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <KeyRoundIcon className="size-8 text-primary/60" />
              {t("access.noRoles")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("access.roleName")}</TableHead>
                  <TableHead>{t("access.permissions")}</TableHead>
                  <TableHead className="hidden md:table-cell">{t("access.admins")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {roles.data?.roles.map((role) => (
                  <TableRow key={role.id}>
                    <TableCell>
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{role.name}</span>
                        {role.own_users_only && (
                          <Badge variant="outline">
                            <UsersIcon /> {t("access.ownUsersTag")}
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex max-w-md flex-wrap gap-1">
                        {role.permissions
                          .filter((name) => !name.endsWith(".read") || !role.permissions.includes(name.replace(".read", ".write")))
                          .map((name) => (
                            <Badge key={name} variant={name.endsWith(".write") ? "default" : "outline"}>
                              {t(`access.section.${name.split(".")[0]}`)}
                              {name.endsWith(".write") ? "" : ` · ${t("access.level.read")}`}
                            </Badge>
                          ))}
                        {role.permissions.length === 0 && <span className="text-xs text-muted-foreground">{t("common.none")}</span>}
                      </div>
                    </TableCell>
                    <TableCell className="tabular hidden md:table-cell">{role.admins}</TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button variant="ghost" size="icon" aria-label={t("common.edit")} onClick={() => onEdit(role)}>
                          <PencilIcon />
                        </Button>
                        <Confirm
                          title={t("access.deleteRoleTitle")}
                          body={role.admins > 0 ? t("access.deleteRoleUsed", { name: role.name, count: role.admins }) : t("access.deleteRoleBody", { name: role.name })}
                          action={t("common.delete")}
                          onConfirm={async () => {
                            await del(`roles/${role.id}`);
                            await client.invalidateQueries({ queryKey: ["roles"] });
                          }}
                          trigger={(confirm) => (
                            <Button variant="ghost" size="icon" aria-label={t("common.delete")} onClick={confirm}>
                              <Trash2Icon className="text-destructive" />
                            </Button>
                          )}
                        />
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </>
  );
}
