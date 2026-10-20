import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { PencilIcon, PlusIcon, Trash2Icon, WaypointsIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { InboundDialog } from "@/components/inbound-dialog";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api, del, put, type Certificate, type Inbound } from "@/lib/api";
import { summary } from "@/lib/inbound-form";
import { useOpenFromLink } from "@/lib/open-from-link";

export function useCertificates() {
  return useQuery({ queryKey: ["certificates"], queryFn: () => api<Certificate[]>("certificates") });
}

export function InboundsPage() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const inbounds = useQuery({ queryKey: ["inbounds"], queryFn: () => api<Inbound[]>("inbounds") });
  const certificates = useCertificates();
  const [editing, setEditing] = useState<Inbound | undefined>();
  const [open, setOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  useOpenFromLink("inbound", () => setOpen(true));

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 4000);
    return () => clearTimeout(timer);
  }, [notice]);

  const refresh = async () => {
    await client.invalidateQueries({ queryKey: ["inbounds"] });
    await client.invalidateQueries({ queryKey: ["health"] });
  };
  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: number; enabled: boolean }) =>
      put<{ applied: string }>(`inbounds/${id}/enabled`, { enabled }),
    onSuccess: async (result) => {
      setNotice(result.applied);
      await refresh();
    },
  });

  const number = (value: number) => new Intl.NumberFormat(i18n.language, { useGrouping: false }).format(value);

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("inbounds.title")} subtitle={t("inbounds.subtitle")} />
        <Button
          onClick={() => {
            setEditing(undefined);
            setOpen(true);
          }}
        >
          <PlusIcon /> {t("inbounds.add")}
        </Button>
      </div>

      {notice && (
        <div className="mb-4 rounded-md border border-success/30 bg-success/10 px-3 py-2 text-sm text-success">
          {t(`inbounds.applied_${notice}`)}
        </div>
      )}
      <div className="mb-4 grid gap-2">
        <FormError error={toggle.error} />
        <FormError error={inbounds.error} />
      </div>

      <Card>
        <CardContent className="p-0">
          {inbounds.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-9" />
              <Skeleton className="h-9" />
            </div>
          ) : inbounds.data?.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <WaypointsIcon className="size-8 text-primary/60" />
              {t("inbounds.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("inbounds.remark")}</TableHead>
                  <TableHead>{t("inbounds.protocol")}</TableHead>
                  <TableHead>{t("inbounds.port")}</TableHead>
                  <TableHead className="hidden sm:table-cell">{t("inbounds.transport")}</TableHead>
                  <TableHead>{t("common.enabled")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {inbounds.data?.map((inbound) => (
                  <TableRow key={inbound.id}>
                    <TableCell>
                      <div className="font-medium">{inbound.remark}</div>
                      <div className="text-xs text-muted-foreground" dir="ltr">
                        {inbound.tag}
                      </div>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">{inbound.protocol}</Badge>
                    </TableCell>
                    <TableCell className="tabular">{number(inbound.port)}</TableCell>
                    <TableCell className="hidden text-muted-foreground sm:table-cell" dir="ltr">
                      {summary(inbound)}
                    </TableCell>
                    <TableCell>
                      <Switch
                        label={t("common.enabled")}
                        checked={inbound.enabled}
                        disabled={toggle.isPending}
                        onCheckedChange={(enabled) => toggle.mutate({ id: inbound.id, enabled })}
                      />
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={t("common.edit")}
                          onClick={() => {
                            setEditing(inbound);
                            setOpen(true);
                          }}
                        >
                          <PencilIcon />
                        </Button>
                        <Confirm
                          title={t("inbounds.deleteTitle")}
                          body={t("inbounds.deleteBody", { name: inbound.remark })}
                          action={t("common.delete")}
                          onConfirm={async () => {
                            const applied = await del<string>(`inbounds/${inbound.id}`);
                            setNotice(applied);
                            await refresh();
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

      <InboundDialog
        open={open}
        onOpenChange={setOpen}
        inbound={editing}
        certificates={certificates.data ?? []}
        onSaved={setNotice}
      />
    </>
  );
}
