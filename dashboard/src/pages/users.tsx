// Layout of the users table and its bulk bar adapted from PasarGuard's dashboard
// (features/users/components/columns.tsx, bulk-actions-bar.tsx), AGPL-3.0.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  BanIcon,
  BarChart3Icon,
  BoxesIcon,
  CalendarPlusIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  DatabaseIcon,
  KeyRoundIcon,
  MoreHorizontalIcon,
  PencilIcon,
  PlayIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SearchIcon,
  Trash2Icon,
  UnplugIcon,
  UsersIcon,
  XIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AmountDialog } from "@/components/amount-dialog";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { GroupPicker, useGroups } from "@/components/pickers";
import { OnlineDot, StatusBadge, UsageBar } from "@/components/user-cells";
import { UsageDialog } from "@/components/usage-dialog";
import { UserDialog } from "@/components/user-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api, del, post, type BulkAction, type User, type UserCounts, type UserPage } from "@/lib/api";
import { GB, statuses } from "@/lib/users";
import { cn } from "@/lib/utils";

const PAGE = 25;

/** `value`, once it has stopped changing for a moment. */
function useSettled<T>(value: T, ms = 300): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

export function useUserCounts() {
  return useQuery({ queryKey: ["users", "counts"], queryFn: () => api<UserCounts>("users/counts"), refetchInterval: 10_000 });
}

export function UsersPage() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const groups = useGroups();
  const counts = useUserCounts();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [group, setGroup] = useState("");
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<number[]>([]);
  const [editing, setEditing] = useState<User | undefined>();
  const [dialog, setDialog] = useState(false);
  const [usageOf, setUsageOf] = useState<User | null>(null);
  const [amount, setAmount] = useState<"extend" | "add_data" | null>(null);
  const [regroup, setRegroup] = useState(false);
  const query = useSettled(search.trim());

  useEffect(() => {
    setOffset(0);
    setSelected([]);
  }, [query, status, group]);

  const params = new URLSearchParams({ offset: String(offset), limit: String(PAGE) });
  if (query) params.set("search", query);
  if (status) params.set("status", status);
  if (group) params.set("group", group);
  const users = useQuery({
    queryKey: ["users", "list", params.toString()],
    queryFn: () => api<UserPage>(`users?${params}`),
    placeholderData: keepPreviousData,
    refetchInterval: 10_000,
  });

  const refresh = () => client.invalidateQueries({ queryKey: ["users"] });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: number; action: "reset" | "revoke" | "end-sessions" }) => post(`users/${id}/${action}`),
    onSuccess: refresh,
  });
  const bulk = async (action: BulkAction) => {
    await post("users/bulk", { ids: selected, ...action });
    setSelected([]);
    await refresh();
  };
  const bulkAct = useMutation({ mutationFn: bulk });

  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);
  const rows = users.data?.users ?? [];
  const total = users.data?.total ?? 0;
  const allOnPage = rows.length > 0 && rows.every((user) => selected.includes(user.id));
  const open = (user?: User) => {
    setEditing(user);
    setDialog(true);
  };

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("users.title")} subtitle={t("users.subtitle")} />
        <Button onClick={() => open()}>
          <PlusIcon /> {t("users.add")}
        </Button>
      </div>

      <div className="-mx-4 mb-4 flex gap-2 overflow-x-auto px-4 pb-1 sm:mx-0 sm:flex-wrap sm:px-0 sm:pb-0">
        {(["total", "online", ...statuses] as const).map((key) => {
          const filter = key === "total" || key === "online" ? "" : key;
          const active = key !== "online" && status === filter;
          return (
            <button
              key={key}
              type="button"
              disabled={key === "online"}
              onClick={() => setStatus(filter)}
              className={cn(
                "flex shrink-0 items-center gap-2 rounded-lg border px-3 py-1.5 text-sm whitespace-nowrap transition-colors enabled:hover:bg-accent",
                active && "border-primary/50 bg-primary/10",
              )}
            >
              {key === "online" && <span className="size-2 rounded-full bg-success" />}
              <span className="text-muted-foreground">{key === "total" ? t("users.all") : key === "online" ? t("users.online") : t(`users.status.${key}`)}</span>
              <span className="tabular font-semibold">{counts.data ? number(counts.data[key] ?? 0) : "–"}</span>
            </button>
          );
        })}
      </div>

      <div className="mb-4 flex flex-col gap-2 sm:flex-row">
        <div className="relative flex-1">
          <SearchIcon className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("users.search")} className="ps-9" aria-label={t("users.search")} />
        </div>
        <div className="grid grid-cols-[1fr_1fr_auto] gap-2 sm:flex">
          <Select value={status} onChange={(e) => setStatus(e.target.value)} aria-label={t("users.statusLabel")} className="sm:w-36">
            <option value="">{t("users.anyStatus")}</option>
            {statuses.map((value) => (
              <option key={value} value={value}>
                {t(`users.status.${value}`)}
              </option>
            ))}
          </Select>
          <Select value={group} onChange={(e) => setGroup(e.target.value)} aria-label={t("users.groups")} className="sm:w-36">
            <option value="">{t("users.anyGroup")}</option>
            {(groups.data ?? []).map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </Select>
          <Button variant="outline" size="icon" aria-label={t("users.refresh")} onClick={() => void refresh()}>
            <RefreshCwIcon className={cn(users.isFetching && "animate-spin")} />
          </Button>
        </div>
      </div>

      {selected.length > 0 && (
        <div className="sticky top-16 z-10 mb-4 flex flex-wrap items-center gap-1 rounded-lg border border-primary/40 bg-card p-2 shadow-lg">
          <span className="px-2 text-sm font-medium">{t("users.selected", { count: selected.length })}</span>
          <Button variant="ghost" size="sm" onClick={() => bulkAct.mutate({ action: "enable" })}>
            <PlayIcon /> {t("users.enable")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => bulkAct.mutate({ action: "disable" })}>
            <BanIcon /> {t("users.disable")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setAmount("extend")}>
            <CalendarPlusIcon /> {t("users.extend")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setAmount("add_data")}>
            <DatabaseIcon /> {t("users.addData")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setRegroup(true)}>
            <BoxesIcon /> {t("users.setGroups")}
          </Button>
          <Confirm
            title={t("users.resetTitle")}
            body={t("users.bulkResetBody", { count: selected.length })}
            action={t("users.resetUsage")}
            onConfirm={() => bulk({ action: "reset" })}
            trigger={(confirm) => (
              <Button variant="ghost" size="sm" onClick={confirm}>
                <RotateCcwIcon /> {t("users.resetUsage")}
              </Button>
            )}
          />
          <Confirm
            title={t("users.revokeTitle")}
            body={t("users.bulkRevokeBody", { count: selected.length })}
            action={t("users.revoke")}
            onConfirm={() => bulk({ action: "revoke" })}
            trigger={(confirm) => (
              <Button variant="ghost" size="sm" onClick={confirm}>
                <KeyRoundIcon /> {t("users.revoke")}
              </Button>
            )}
          />
          <Confirm
            title={t("users.deleteTitle")}
            body={t("users.bulkDeleteBody", { count: selected.length })}
            action={t("common.delete")}
            onConfirm={() => bulk({ action: "delete" })}
            trigger={(confirm) => (
              <Button variant="ghost" size="sm" className="text-destructive" onClick={confirm}>
                <Trash2Icon /> {t("common.delete")}
              </Button>
            )}
          />
          <Button variant="ghost" size="icon" className="ms-auto size-8" aria-label={t("common.cancel")} onClick={() => setSelected([])}>
            <XIcon />
          </Button>
        </div>
      )}

      <div className="mb-4 grid gap-2">
        <FormError error={users.error} />
        <FormError error={act.error} />
        <FormError error={bulkAct.error} />
      </div>

      <Card>
        <CardContent className="p-0">
          {users.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : rows.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <UsersIcon className="size-8 text-primary/60" />
              {query || status || group ? t("users.noMatch") : t("users.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-10">
                    <Checkbox
                      aria-label={t("users.selectPage")}
                      checked={allOnPage}
                      onChange={(e) =>
                        setSelected(
                          e.target.checked
                            ? [...new Set([...selected, ...rows.map((user) => user.id)])]
                            : selected.filter((id) => !rows.some((user) => user.id === id)),
                        )
                      }
                    />
                  </TableHead>
                  <TableHead>{t("users.username")}</TableHead>
                  <TableHead>{t("users.statusLabel")}</TableHead>
                  <TableHead className="hidden md:table-cell">{t("users.usage")}</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((user) => (
                  <TableRow key={user.id} className="cursor-pointer" onClick={() => open(user)}>
                    <TableCell onClick={(e) => e.stopPropagation()}>
                      <Checkbox
                        aria-label={user.username}
                        checked={selected.includes(user.id)}
                        onChange={(e) => setSelected(e.target.checked ? [...selected, user.id] : selected.filter((id) => id !== user.id))}
                      />
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-2.5">
                        <OnlineDot user={user} />
                        <div className="min-w-0">
                          <div className="flex items-baseline gap-1.5">
                            <span className="truncate font-medium" dir="ltr">
                              {user.username}
                            </span>
                            <span className="hidden font-mono text-[10px] text-muted-foreground/70 md:inline">#{user.id}</span>
                          </div>
                          {user.note && <div className="max-w-48 truncate text-xs text-muted-foreground">{user.note}</div>}
                        </div>
                      </div>
                      <div className="mt-2 md:hidden">
                        <UsageBar user={user} />
                      </div>
                    </TableCell>
                    <TableCell>
                      <StatusBadge user={user} />
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <UsageBar user={user} />
                    </TableCell>
                    <TableCell onClick={(e) => e.stopPropagation()}>
                      <UserActions
                        user={user}
                        onEdit={() => open(user)}
                        onUsage={() => setUsageOf(user)}
                        onAction={(action) => act.mutate({ id: user.id, action })}
                        onDelete={async () => {
                          await del(`users/${user.id}`);
                          setSelected((ids) => ids.filter((id) => id !== user.id));
                          await refresh();
                        }}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {total > PAGE && (
        <div className="mt-4 flex items-center justify-between gap-4 text-sm text-muted-foreground">
          <span className="tabular">
            {t("users.range", { from: number(offset + 1), to: number(Math.min(offset + PAGE, total)), total: number(total) })}
          </span>
          <div className="flex gap-1">
            <Button variant="outline" size="icon" aria-label={t("users.previous")} disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))}>
              <ChevronRightIcon className="ltr:rotate-180" />
            </Button>
            <Button variant="outline" size="icon" aria-label={t("users.next")} disabled={offset + PAGE >= total} onClick={() => setOffset(offset + PAGE)}>
              <ChevronLeftIcon className="ltr:rotate-180" />
            </Button>
          </div>
        </div>
      )}

      <UserDialog open={dialog} onOpenChange={setDialog} user={editing} />
      <UsageDialog user={usageOf} onOpenChange={(isOpen) => !isOpen && setUsageOf(null)} />
      <RegroupDialog
        open={regroup}
        onOpenChange={setRegroup}
        count={selected.length}
        onSubmit={(group_ids) => bulk({ action: "set_groups", group_ids })}
      />
      <AmountDialog
        open={amount !== null}
        onOpenChange={(isOpen) => !isOpen && setAmount(null)}
        title={amount === "extend" ? t("users.extendTitle", { count: selected.length }) : t("users.addDataTitle", { count: selected.length })}
        label={amount === "extend" ? t("users.extendDays") : t("users.addDataGb")}
        onSubmit={(value) =>
          bulk(amount === "extend" ? { action: "extend", days: Math.round(value) } : { action: "add_data", bytes: Math.round(value * GB) })
        }
      />
    </>
  );
}

/** Put the selected users in exactly the groups picked. */
function RegroupDialog({
  open,
  onOpenChange,
  count,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  count: number;
  onSubmit: (groupIds: number[]) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [groupIds, setGroupIds] = useState<number[]>([]);
  const save = useMutation({ mutationFn: () => onSubmit(groupIds), onSuccess: () => onOpenChange(false) });
  useEffect(() => {
    if (open) {
      setGroupIds([]);
      save.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("users.setGroupsTitle", { count })} className="max-w-md">
        <div className="grid gap-4">
          <GroupPicker value={groupIds} onChange={setGroupIds} hint={t("users.setGroupsHint")} />
          <FormError error={save.error} />
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button disabled={save.isPending} onClick={() => save.mutate()}>
              {t("common.apply")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function UserActions({
  user,
  onEdit,
  onUsage,
  onAction,
  onDelete,
}: {
  user: User;
  onEdit: () => void;
  onUsage: () => void;
  onAction: (action: "reset" | "revoke" | "end-sessions") => void;
  onDelete: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  return (
    <Confirm
      title={t("users.deleteTitle")}
      body={t("users.deleteBody", { name: user.username })}
      action={t("common.delete")}
      onConfirm={onDelete}
      trigger={(confirmDelete) => (
        <Confirm
          title={t("users.revokeTitle")}
          body={t("users.revokeBody", { name: user.username })}
          action={t("users.revoke")}
          onConfirm={async () => onAction("revoke")}
          trigger={(confirmRevoke) => (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon" aria-label={t("common.actions")}>
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={onEdit}>
                  <PencilIcon /> {t("common.edit")}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onUsage}>
                  <BarChart3Icon /> {t("users.usageChart")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => onAction("reset")}>
                  <RotateCcwIcon /> {t("users.resetUsage")}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => onAction("end-sessions")} disabled={!user.online}>
                  <UnplugIcon /> {t("users.endSessions")}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={confirmRevoke}>
                  <KeyRoundIcon /> {t("users.revoke")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={confirmDelete} className="text-destructive data-[highlighted]:text-destructive">
                  <Trash2Icon /> {t("common.delete")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        />
      )}
    />
  );
}
