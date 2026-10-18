import { useMutation, useQueryClient } from "@tanstack/react-query";
import { BoxesIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { InboundPicker, useGroups, useInbounds } from "@/components/pickers";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { del, post, put, type AccessGroup } from "@/lib/api";

function GroupDialog({ open, onOpenChange, group }: { open: boolean; onOpenChange: (open: boolean) => void; group?: AccessGroup }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [all, setAll] = useState(false);
  const [inbounds, setInbounds] = useState<number[]>([]);
  useEffect(() => {
    if (!open) return;
    setName(group?.name ?? "");
    setAll(group?.all_inbounds ?? false);
    setInbounds(group?.inbound_ids ?? []);
    save.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, group]);
  const save = useMutation({
    mutationFn: () => {
      const body = { name: name.trim(), all_inbounds: all, inbound_ids: all ? [] : inbounds };
      return group ? put(`groups/${group.id}`, body) : post("groups", body);
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["groups"] });
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={group ? t("groups.editTitle") : t("groups.newTitle")}>
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("groups.name")} value={name} onChange={(e) => setName(e.target.value)} required maxLength={64} />
          <SwitchField label={t("groups.allInbounds")} hint={t("groups.allInboundsHint")} checked={all} onCheckedChange={setAll} />
          {!all && <InboundPicker value={inbounds} onChange={setInbounds} />}
          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {group ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function GroupsPage() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const groups = useGroups();
  const inbounds = useInbounds();
  const [editing, setEditing] = useState<AccessGroup | undefined>();
  const [open, setOpen] = useState(false);
  const remark = (id: number) => inbounds.data?.find((inbound) => inbound.id === id)?.remark ?? `#${id}`;
  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("groups.title")} subtitle={t("groups.subtitle")} />
        <Button
          onClick={() => {
            setEditing(undefined);
            setOpen(true);
          }}
        >
          <PlusIcon /> {t("groups.add")}
        </Button>
      </div>
      <div className="mb-4">
        <FormError error={groups.error} />
      </div>
      <Card>
        <CardContent className="p-0">
          {groups.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-9" />
              <Skeleton className="h-9" />
            </div>
          ) : groups.data?.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <BoxesIcon className="size-8 text-primary/60" />
              {t("groups.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("groups.name")}</TableHead>
                  <TableHead>{t("groups.inbounds")}</TableHead>
                  <TableHead>{t("groups.users")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {groups.data?.map((group) => (
                  <TableRow key={group.id}>
                    <TableCell className="font-medium">{group.name}</TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {group.all_inbounds ? (
                          <Badge>{t("groups.all")}</Badge>
                        ) : group.inbound_ids.length === 0 ? (
                          <span className="text-xs text-muted-foreground">{t("common.none")}</span>
                        ) : (
                          group.inbound_ids.map((id) => (
                            <Badge key={id} variant="outline">
                              {remark(id)}
                            </Badge>
                          ))
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="tabular">{number(group.users)}</TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={t("common.edit")}
                          onClick={() => {
                            setEditing(group);
                            setOpen(true);
                          }}
                        >
                          <PencilIcon />
                        </Button>
                        <Confirm
                          title={t("groups.deleteTitle")}
                          body={t("groups.deleteBody", { name: group.name, count: group.users })}
                          action={t("common.delete")}
                          onConfirm={async () => {
                            await del(`groups/${group.id}`);
                            await client.invalidateQueries({ queryKey: ["groups"] });
                            await client.invalidateQueries({ queryKey: ["users"] });
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
      <GroupDialog open={open} onOpenChange={setOpen} group={editing} />
    </>
  );
}
