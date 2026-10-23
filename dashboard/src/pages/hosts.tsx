import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NetworkIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { Can } from "@/components/can";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api, del, post, put, type Host, type Inbound } from "@/lib/api";
import { fingerprints } from "@/lib/inbound-form";
import { useOpenFromLink } from "@/lib/open-from-link";

type HostForm = Omit<Host, "id">;

const blank = (inboundId: number): HostForm => ({
  inbound_id: inboundId,
  remark: "",
  address: "",
  port: null,
  sni: "",
  host: "",
  path: "",
  alpn: "",
  fingerprint: "",
  allow_insecure: false,
  enabled: true,
  priority: 0,
});

function HostDialog({
  open,
  onOpenChange,
  host,
  inbounds,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  host?: Host;
  inbounds: Inbound[];
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [form, setForm] = useState<HostForm>(blank(0));
  useEffect(() => {
    if (open) setForm(host ? { ...host } : blank(inbounds[0]?.id ?? 0));
  }, [open, host, inbounds]);
  const set = (patch: Partial<HostForm>) => setForm((current) => ({ ...current, ...patch }));
  const save = useMutation({
    mutationFn: () => (host ? put(`hosts/${host.id}`, form) : post("hosts", form)),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["hosts"] });
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={host ? t("hosts.editTitle") : t("hosts.add")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              label={t("hosts.inbound")}
              value={String(form.inbound_id)}
              onChange={(e) => set({ inbound_id: Number(e.target.value) })}
              options={inbounds.map((i) => ({ value: String(i.id), label: `${i.remark} (${i.protocol}:${i.port})` }))}
            />
            <Field label={t("hosts.remark")} value={form.remark} onChange={(e) => set({ remark: e.target.value })} required />
            <Field
              label={t("hosts.address")}
              value={form.address}
              onChange={(e) => set({ address: e.target.value })}
              placeholder="example.com"
              dir="ltr"
              required
            />
            <Field
              label={t("hosts.port")}
              hint={t("hosts.portHint")}
              type="number"
              min={1}
              max={65535}
              value={form.port ?? ""}
              onChange={(e) => set({ port: e.target.value ? Number(e.target.value) : null })}
              dir="ltr"
            />
            <Field label={t("hosts.sni")} value={form.sni} onChange={(e) => set({ sni: e.target.value })} dir="ltr" />
            <Field label={t("hosts.host")} value={form.host} onChange={(e) => set({ host: e.target.value })} dir="ltr" />
            <Field label={t("hosts.path")} value={form.path} onChange={(e) => set({ path: e.target.value })} dir="ltr" />
            <SelectField
              label={t("hosts.alpn")}
              value={form.alpn}
              onChange={(e) => set({ alpn: e.target.value })}
              options={[
                { value: "", label: t("hosts.inherit") },
                { value: "h2", label: "h2" },
                { value: "http/1.1", label: "http/1.1" },
                { value: "h2,http/1.1", label: "h2, http/1.1" },
                { value: "h3", label: "h3" },
              ]}
            />
            <SelectField
              label={t("hosts.fingerprint")}
              value={form.fingerprint}
              onChange={(e) => set({ fingerprint: e.target.value })}
              options={[{ value: "", label: t("hosts.inherit") }, ...fingerprints.map((f) => ({ value: f, label: f }))]}
            />
          </div>
          <SwitchField
            label={t("hosts.allowInsecure")}
            hint={t("hosts.allowInsecureHint")}
            checked={form.allow_insecure}
            onCheckedChange={(allow_insecure) => set({ allow_insecure })}
          />
          <SwitchField label={t("common.enabled")} checked={form.enabled} onCheckedChange={(enabled) => set({ enabled })} />
          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {host ? t("common.save") : t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function HostsPage() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const hosts = useQuery({ queryKey: ["hosts"], queryFn: () => api<Host[]>("hosts") });
  const inbounds = useQuery({ queryKey: ["inbounds"], queryFn: () => api<Inbound[]>("inbounds") });
  const [editing, setEditing] = useState<Host | undefined>();
  const [open, setOpen] = useState(false);
  useOpenFromLink("host", () => setOpen(true));
  const byId = new Map((inbounds.data ?? []).map((inbound) => [inbound.id, inbound]));
  const noInbound = inbounds.data?.length === 0;

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("hosts.title")} subtitle={t("hosts.subtitle")} />
        <Can perm="hosts.write">
<Button
          disabled={noInbound}
          title={noInbound ? t("hosts.needInbound") : undefined}
          onClick={() => {
            setEditing(undefined);
            setOpen(true);
          }}
        >
          <PlusIcon /> {t("hosts.add")}
        </Button>
</Can>
      </div>
      <FormError error={hosts.error} />
      <Card>
        <CardContent className="p-0">
          {hosts.isPending ? (
            <div className="p-5">
              <Skeleton className="h-9" />
            </div>
          ) : hosts.data?.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <NetworkIcon className="size-8 text-primary/60" />
              {noInbound ? t("hosts.needInbound") : t("hosts.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("hosts.remark")}</TableHead>
                  <TableHead>{t("hosts.inbound")}</TableHead>
                  <TableHead>{t("hosts.address")}</TableHead>
                  <TableHead>{t("common.enabled")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {hosts.data?.map((host) => {
                  const inbound = byId.get(host.inbound_id);
                  return (
                    <TableRow key={host.id}>
                      <TableCell className="font-medium">{host.remark}</TableCell>
                      <TableCell>{inbound?.remark ?? "—"}</TableCell>
                      <TableCell dir="ltr" className="text-start">
                        {host.address}:{host.port ?? inbound?.port}
                        {host.sni && <span className="ms-2 text-xs text-muted-foreground">sni={host.sni}</span>}
                      </TableCell>
                      <TableCell>
                        <Badge variant={host.enabled ? "success" : "secondary"}>
                          {host.enabled ? t("common.enabled") : t("common.disabled")}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Can perm="hosts.write">
<div className="flex justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={t("common.edit")}
                            onClick={() => {
                              setEditing(host);
                              setOpen(true);
                            }}
                          >
                            <PencilIcon />
                          </Button>
                          <Confirm
                            title={t("hosts.deleteTitle")}
                            body={t("hosts.deleteBody", { name: host.remark })}
                            action={t("common.delete")}
                            onConfirm={async () => {
                              await del(`hosts/${host.id}`);
                              await client.invalidateQueries({ queryKey: ["hosts"] });
                            }}
                            trigger={(confirm) => (
                              <Button variant="ghost" size="icon" aria-label={t("common.delete")} onClick={confirm}>
                                <Trash2Icon className="text-destructive" />
                              </Button>
                            )}
                          />
                        </div>
</Can>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
      <HostDialog open={open} onOpenChange={setOpen} host={editing} inbounds={inbounds.data ?? []} />
    </>
  );
}
