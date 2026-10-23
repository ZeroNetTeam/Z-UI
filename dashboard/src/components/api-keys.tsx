import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRoundIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { CopyButton } from "@/components/copy-button";
import { FormError } from "@/components/form-error";
import { PermissionMatrix } from "@/components/permission-matrix";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Segmented } from "@/components/ui/segmented";
import { SelectField } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { TextareaField } from "@/components/ui/textarea";
import { api, del, post, type ApiKey, type ApiKeyCreated, type ApiKeyPayload } from "@/lib/api";
import { curl } from "@/lib/openapi";
import { minimal, sections, useMe } from "@/lib/permissions";
import { formatDate, relative } from "@/lib/users";

type Scope = "full" | "read" | "custom";

const expiries = [0, 7, 30, 90, 365];

/** The key just made, in full: this is the only time it is shown. */
function Secret({ created, onClose }: { created: ApiKeyCreated | null; onClose: () => void }) {
  const { t } = useTranslation();
  const address = new URL("api/auth/me", document.baseURI).href;
  return (
    <Dialog open={created !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent title={t("developer.keyMade")} description={t("developer.keyMadeBody")} className="max-w-xl">
        {created && (
          <div className="grid gap-4">
            <div className="grid gap-2">
              <code className="block rounded-md border bg-muted/40 p-3 font-mono text-sm break-all select-all" dir="ltr">
                {created.key}
              </code>
              <div>
                <CopyButton text={created.key} label={t("developer.copyKey")} />
              </div>
            </div>
            <div className="grid gap-2">
              <span className="text-xs font-medium text-muted-foreground">{t("developer.tryWithCurl")}</span>
              <pre className="overflow-x-auto rounded-md border bg-muted/40 p-3 font-mono text-xs" dir="ltr">
                {curl("get", address, null, true)}
              </pre>
            </div>
            <div className="flex justify-end">
              <Button onClick={onClose}>{t("developer.keySaved")}</Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function NewKeyDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (key: ApiKeyCreated) => void;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const me = useMe();
  const [name, setName] = useState("");
  const [scope, setScope] = useState<Scope>("read");
  const [custom, setCustom] = useState<string[]>([]);
  const [days, setDays] = useState("0");
  const [ips, setIps] = useState("");
  const mine = me?.permissions ?? [];
  // What a key may be limited to: reading, for each part its admin may read.
  const reads = sections.filter((section) => mine.includes(`${section}.read`)).map((section) => `${section}.read`);

  useEffect(() => {
    if (!open) return;
    setName("");
    setScope("read");
    setCustom([]);
    setDays("0");
    setIps("");
    save.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const permissions = scope === "full" ? "*" : scope === "read" ? reads : minimal(custom);
  const save = useMutation({
    mutationFn: () => {
      const body: ApiKeyPayload = {
        name: name.trim(),
        permissions,
        allowed_ips: ips.split(/[\s,]+/).filter(Boolean),
        expires_at: Number(days) > 0 ? Math.floor(Date.now() / 1000) + Number(days) * 86_400 : null,
      };
      return post<ApiKeyCreated>("api-keys", body);
    },
    onSuccess: async (created) => {
      await client.invalidateQueries({ queryKey: ["api-keys"] });
      onOpenChange(false);
      onCreated(created);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("developer.newKey")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("developer.keyName")} hint={t("developer.keyNameHint")} value={name} onChange={(e) => setName(e.target.value)} required maxLength={64} />
          <div className="grid gap-2">
            <span className="text-sm font-medium">{t("developer.scope")}</span>
            <Segmented
              label={t("developer.scope")}
              value={scope}
              onChange={setScope}
              options={[
                { value: "read", label: t("developer.scopeRead") },
                { value: "custom", label: t("developer.scopeCustom") },
                { value: "full", label: t("developer.scopeFull") },
              ]}
            />
            <p className="text-xs text-muted-foreground">{t(`developer.scopeHint_${scope}`)}</p>
          </div>
          {scope === "custom" && <PermissionMatrix value={custom} onChange={setCustom} limit={mine} />}
          <SelectField
            label={t("developer.expires")}
            value={days}
            onChange={(e) => setDays(e.target.value)}
            options={expiries.map((value) => ({ value: String(value), label: value === 0 ? t("developer.never") : t("common.days", { count: value }) }))}
          />
          <TextareaField
            label={t("developer.allowedIps")}
            hint={t("developer.allowedIpsHint")}
            value={ips}
            onChange={(e) => setIps(e.target.value)}
            dir="ltr"
            spellCheck={false}
            placeholder={"203.0.113.7\n10.0.0.0/8"}
            className="min-h-16 font-mono text-xs"
          />
          <FormError error={save.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending || (Array.isArray(permissions) && permissions.length === 0)}>
              {t("common.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** The signed-in admin's own API keys. */
export function KeysTab() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const keys = useQuery({ queryKey: ["api-keys"], queryFn: () => api<ApiKey[]>("api-keys") });
  const [dialog, setDialog] = useState(false);
  const [created, setCreated] = useState<ApiKeyCreated | null>(null);

  const scope = (key: ApiKey) => {
    if (key.full) return <Badge>{t("developer.scopeFull")}</Badge>;
    const writes = key.permissions.filter((name) => name.endsWith(".write")).length;
    return (
      <Badge variant="outline">
        {writes > 0 ? t("developer.scopeSome", { count: key.permissions.length }) : t("developer.scopeRead")}
      </Badge>
    );
  };

  return (
    <>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <p className="max-w-2xl text-sm text-muted-foreground">{t("developer.keysBody")}</p>
        <Button onClick={() => setDialog(true)}>
          <PlusIcon /> {t("developer.newKey")}
        </Button>
      </div>
      <div className="mb-4">
        <FormError error={keys.error} />
      </div>
      <Card>
        <CardContent className="p-0">
          {keys.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-10" />
              <Skeleton className="h-10" />
            </div>
          ) : keys.data?.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <KeyRoundIcon className="size-8 text-primary/60" />
              {t("developer.noKeys")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("developer.keyName")}</TableHead>
                  <TableHead>{t("developer.scope")}</TableHead>
                  <TableHead className="hidden md:table-cell">{t("developer.expires")}</TableHead>
                  <TableHead className="hidden lg:table-cell">{t("developer.lastUsed")}</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {keys.data?.map((key) => (
                  <TableRow key={key.id}>
                    <TableCell>
                      <div className="font-medium">{key.name}</div>
                      <div className="font-mono text-xs text-muted-foreground" dir="ltr">
                        {key.prefix}…
                      </div>
                      {key.allowed_ips.length > 0 && (
                        <div className="mt-0.5 max-w-56 truncate font-mono text-[11px] text-muted-foreground" dir="ltr" title={key.allowed_ips.join(", ")}>
                          {key.allowed_ips.join(", ")}
                        </div>
                      )}
                    </TableCell>
                    <TableCell>{scope(key)}</TableCell>
                    <TableCell className="hidden text-xs md:table-cell">
                      {key.expires_at === null ? (
                        <span className="text-muted-foreground">{t("developer.never")}</span>
                      ) : key.expired ? (
                        <Badge variant="destructive">{t("developer.expired")}</Badge>
                      ) : (
                        <span title={formatDate(key.expires_at, i18n.language, true)}>{relative(key.expires_at, i18n.language)}</span>
                      )}
                    </TableCell>
                    <TableCell className="hidden text-xs text-muted-foreground lg:table-cell">
                      {key.last_used_at ? (
                        <>
                          {relative(key.last_used_at, i18n.language)}
                          {key.last_used_ip && <span dir="ltr"> · {key.last_used_ip}</span>}
                        </>
                      ) : (
                        t("developer.neverUsed")
                      )}
                    </TableCell>
                    <TableCell>
                      <Confirm
                        title={t("developer.revokeTitle")}
                        body={t("developer.revokeBody", { name: key.name })}
                        action={t("developer.revoke")}
                        onConfirm={async () => {
                          await del(`api-keys/${key.id}`);
                          await client.invalidateQueries({ queryKey: ["api-keys"] });
                        }}
                        trigger={(confirm) => (
                          <Button variant="ghost" size="icon" aria-label={t("developer.revoke")} onClick={confirm}>
                            <Trash2Icon className="text-destructive" />
                          </Button>
                        )}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
      <NewKeyDialog open={dialog} onOpenChange={setDialog} onCreated={setCreated} />
      <Secret created={created} onClose={() => setCreated(null)} />
    </>
  );
}
