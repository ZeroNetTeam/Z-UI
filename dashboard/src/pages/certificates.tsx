import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FileKey2Icon, PlusIcon, Trash2Icon, UploadIcon } from "lucide-react";
import { useState, type ChangeEvent, type FormEvent } from "react";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { TextareaField } from "@/components/ui/textarea";
import { del, post, type Certificate } from "@/lib/api";
import { useOpenFromLink } from "@/lib/open-from-link";
import { useCertificates } from "./inbounds";

function FilePicker({ onText }: { onText: (text: string) => void }) {
  const { t } = useTranslation();
  const read = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (file) void file.text().then(onText);
  };
  return (
    <label className="inline-flex cursor-pointer items-center gap-1.5 text-xs text-primary hover:underline">
      <UploadIcon className="size-3.5" /> {t("certificates.fromFile")}
      <input type="file" accept=".pem,.crt,.cer,.key,.txt" className="sr-only" onChange={read} />
    </label>
  );
}

function AddCertificate({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [cert, setCert] = useState("");
  const [key, setKey] = useState("");
  const add = useMutation({
    mutationFn: () => post<Certificate>("certificates", { name, cert_pem: cert, key_pem: key }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["certificates"] });
      setName("");
      setCert("");
      setKey("");
      onOpenChange(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    add.mutate();
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("certificates.add")} className="max-w-xl">
        <form onSubmit={submit} className="grid gap-4">
          <Field label={t("certificates.name")} hint={t("certificates.nameHint")} value={name} onChange={(e) => setName(e.target.value)} />
          <div className="grid gap-1">
            <TextareaField
              label={t("certificates.cert")}
              value={cert}
              onChange={(e) => setCert(e.target.value)}
              placeholder="-----BEGIN CERTIFICATE-----"
              dir="ltr"
              spellCheck={false}
              className="min-h-28 font-mono text-xs"
              required
            />
            <FilePicker onText={setCert} />
          </div>
          <div className="grid gap-1">
            <TextareaField
              label={t("certificates.key")}
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder="-----BEGIN PRIVATE KEY-----"
              dir="ltr"
              spellCheck={false}
              className="min-h-28 font-mono text-xs"
              required
            />
            <FilePicker onText={setKey} />
          </div>
          <FormError error={add.error} />
          <div className="flex justify-end gap-2 border-t pt-4">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={add.isPending}>
              {t("common.add")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function CertificatesPage() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const certificates = useCertificates();
  const [open, setOpen] = useState(false);
  useOpenFromLink("certificate", () => setOpen(true));
  const now = Date.now() / 1000;
  const date = (seconds: number) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium" }).format(new Date(seconds * 1000));

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("certificates.title")} subtitle={t("certificates.subtitle")} />
        <Can perm="certificates.write">
<Button onClick={() => setOpen(true)}>
          <PlusIcon /> {t("certificates.add")}
        </Button>
</Can>
      </div>
      <FormError error={certificates.error} />
      <Card>
        <CardContent className="p-0">
          {certificates.isPending ? (
            <div className="p-5">
              <Skeleton className="h-9" />
            </div>
          ) : certificates.data?.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <FileKey2Icon className="size-8 text-primary/60" />
              {t("certificates.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("certificates.name")}</TableHead>
                  <TableHead>{t("certificates.domains")}</TableHead>
                  <TableHead>{t("certificates.expires")}</TableHead>
                  <TableHead className="text-end">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {certificates.data?.map((certificate) => {
                  const days = Math.floor((certificate.not_after - now) / 86400);
                  return (
                    <TableRow key={certificate.id}>
                      <TableCell className="font-medium">{certificate.name}</TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1" dir="ltr">
                          {certificate.domains.map((domain) => (
                            <Badge key={domain} variant="secondary">
                              {domain}
                            </Badge>
                          ))}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div>{date(certificate.not_after)}</div>
                        <Badge variant={days < 0 ? "destructive" : days < 14 ? "default" : "success"} className="mt-1">
                          {days < 0
                            ? t("certificates.expired")
                            : t("certificates.daysLeft", { days: new Intl.NumberFormat(i18n.language).format(days) })}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Can perm="certificates.write">
<div className="flex justify-end">
                          <Confirm
                            title={t("certificates.deleteTitle")}
                            body={t("certificates.deleteBody", { name: certificate.name })}
                            action={t("common.delete")}
                            onConfirm={async () => {
                              await del(`certificates/${certificate.id}`);
                              await client.invalidateQueries({ queryKey: ["certificates"] });
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
      <AddCertificate open={open} onOpenChange={setOpen} />
    </>
  );
}
