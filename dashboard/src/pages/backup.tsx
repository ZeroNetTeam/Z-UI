import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArchiveIcon, CircleCheckIcon, DownloadIcon, LockIcon, RotateCcwIcon, Trash2Icon, UploadIcon } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import {
  api,
  del,
  post,
  put,
  upload,
  type BackupMade,
  type BackupSettingsPayload,
  type BackupsView,
  type Restored,
} from "@/lib/api";
import { useAuthChanged } from "@/lib/auth";
import { formatDate, relative } from "@/lib/users";
import { formatBytes } from "@/lib/utils";

const key = ["backups"] as const;

function MakeCard({ view }: { view: BackupsView }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const make = useMutation({
    mutationFn: () => post<BackupMade>("backups"),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
  return (
    <Card>
      <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
        <div>
          <CardTitle>{t("backup.nowTitle")}</CardTitle>
          <CardDescription className="mt-1 max-w-2xl">{t("backup.nowBody")}</CardDescription>
        </div>
        <Button disabled={make.isPending} onClick={() => make.mutate()}>
          <ArchiveIcon /> {make.isPending ? t("backup.making") : t("backup.make")}
        </Button>
      </CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-sm text-muted-foreground">
          {view.last_at ? t("backup.lastAt", { when: relative(view.last_at, i18n.language) }) : t("backup.never")}
        </p>
        <FormError error={make.error} />
        {make.isSuccess && (
          <p className="flex items-center gap-1.5 text-sm text-success" dir="auto">
            <CircleCheckIcon className="size-4" />
            {t("backup.made", { file: make.data.file, size: formatBytes(make.data.bytes, i18n.language), sent: make.data.sent })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function Files({ view }: { view: BackupsView }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("backup.filesTitle")}</CardTitle>
        <CardDescription>{t("backup.filesBody", { keep: view.settings.keep })}</CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        {view.files.length === 0 ? (
          <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
            <ArchiveIcon className="size-8 text-primary/60" />
            {t("backup.noFiles")}
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("backup.file")}</TableHead>
                <TableHead className="hidden sm:table-cell">{t("backup.size")}</TableHead>
                <TableHead className="hidden md:table-cell">{t("backup.when")}</TableHead>
                <TableHead className="text-end">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {view.files.map((file) => (
                <TableRow key={file.name}>
                  <TableCell>
                    <div className="break-all font-mono text-xs" dir="ltr">
                      {file.name}
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {file.encrypted && (
                        <Badge variant="success">
                          <LockIcon /> {t("backup.encrypted")}
                        </Badge>
                      )}
                      {file.before_restore && <Badge variant="outline">{t("backup.beforeRestore")}</Badge>}
                    </div>
                  </TableCell>
                  <TableCell className="tabular hidden sm:table-cell">{formatBytes(file.bytes, i18n.language)}</TableCell>
                  <TableCell className="hidden text-xs text-muted-foreground md:table-cell" title={formatDate(file.at, i18n.language, true)}>
                    {relative(file.at, i18n.language)}
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-1">
                      <Button variant="ghost" size="icon" asChild aria-label={t("backup.download")}>
                        <a href={new URL(`api/backups/${file.name}`, document.baseURI).href} download={file.name}>
                          <DownloadIcon />
                        </a>
                      </Button>
                      <Confirm
                        title={t("backup.deleteTitle")}
                        body={t("backup.deleteBody", { name: file.name })}
                        action={t("common.delete")}
                        onConfirm={async () => {
                          await del(`backups/${file.name}`);
                          await client.invalidateQueries({ queryKey: key });
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
  );
}

function Schedule({ view }: { view: BackupsView }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const stored = view.settings;
  const [enabled, setEnabled] = useState(stored.enabled);
  const [hours, setHours] = useState(String(stored.interval_hours));
  const [keep, setKeep] = useState(String(stored.keep));
  const [telegram, setTelegram] = useState(stored.send_telegram);
  const [usage, setUsage] = useState(stored.include_usage);
  const [passphrase, setPassphrase] = useState("");
  const [clear, setClear] = useState(false);
  const save = useMutation({
    mutationFn: () => {
      const body: BackupSettingsPayload = {
        enabled,
        interval_hours: Math.round(Number(hours) || 0),
        keep: Math.round(Number(keep) || 0),
        send_telegram: telegram,
        include_usage: usage,
        passphrase: passphrase.trim(),
        clear_passphrase: clear,
      };
      return put<BackupsView>("backups/settings", body);
    },
    onSuccess: (saved) => {
      client.setQueryData(key, saved);
      setPassphrase("");
      setClear(false);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("backup.scheduleTitle")}</CardTitle>
        <CardDescription>{t("backup.scheduleBody")}</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid gap-4">
          <SwitchField label={t("backup.auto")} hint={t("backup.autoHint")} checked={enabled} onCheckedChange={setEnabled} />
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t("backup.every")} type="number" min={1} max={720} value={hours} onChange={(e) => setHours(e.target.value)} dir="ltr" hint={t("backup.everyHint")} />
            <Field label={t("backup.keep")} type="number" min={1} max={100} value={keep} onChange={(e) => setKeep(e.target.value)} dir="ltr" hint={t("backup.keepHint")} />
          </div>
          <SwitchField label={t("backup.toTelegram")} hint={t("backup.toTelegramHint")} checked={telegram} onCheckedChange={setTelegram} />
          <SwitchField label={t("backup.usage")} hint={t("backup.usageHint")} checked={usage} onCheckedChange={setUsage} />
          <Field
            label={t("backup.passphrase")}
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            dir="ltr"
            autoComplete="new-password"
            placeholder={stored.passphrase_set ? t("notify.secretSet") : ""}
            disabled={clear}
            hint={t("backup.passphraseHint")}
          />
          {stored.passphrase_set && (
            <SwitchField label={t("backup.clearPassphrase")} hint={t("backup.clearPassphraseHint")} checked={clear} onCheckedChange={setClear} />
          )}
          <FormError error={save.error} />
          {save.isSuccess && <p className="text-sm text-success">{t("settings.saved")}</p>}
          <Button type="submit" className="justify-self-start" disabled={save.isPending}>
            {t("settings.save")}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function Restore() {
  const { t, i18n } = useTranslation();
  const authChanged = useAuthChanged();
  const input = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [asking, setAsking] = useState(false);
  const [done, setDone] = useState<Restored | null>(null);
  const restore = useMutation({
    mutationFn: () => {
      if (!file) throw new Error("no file");
      return upload<Restored>("backups/restore", file, passphrase ? { "X-Backup-Passphrase": passphrase } : {});
    },
    onSuccess: (result) => {
      setAsking(false);
      setDone(result);
      setPassphrase("");
    },
  });
  const finish = async () => {
    // Every session ended with the restore: the page goes back to signing in.
    setDone(null);
    await authChanged();
  };
  return (
    <Card className="border-destructive/30">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <RotateCcwIcon className="size-4 text-destructive" /> {t("backup.restoreTitle")}
        </CardTitle>
        <CardDescription>{t("backup.restoreBody")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        <div className="grid gap-2">
          <input
            ref={input}
            type="file"
            accept=".zbk,.enc,application/octet-stream"
            className="sr-only"
            aria-label={t("backup.pick")}
            onChange={(e) => {
              setFile(e.target.files?.[0] ?? null);
              restore.reset();
            }}
          />
          <div className="flex flex-wrap items-center gap-3">
            <Button type="button" variant="outline" onClick={() => input.current?.click()}>
              <UploadIcon /> {t("backup.pick")}
            </Button>
            {file && (
              <span className="min-w-0 break-all text-sm" dir="ltr">
                {file.name} · {formatBytes(file.size, i18n.language)}
              </span>
            )}
          </div>
        </div>
        <Field
          label={t("backup.restorePassphrase")}
          type="password"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          dir="ltr"
          autoComplete="off"
          hint={t("backup.restorePassphraseHint")}
        />
        <FormError error={restore.error} />
        <Button variant="destructive" className="justify-self-start" disabled={!file || restore.isPending} onClick={() => setAsking(true)}>
          {t("backup.restoreAction")}
        </Button>
      </CardContent>

      <Dialog open={asking} onOpenChange={setAsking}>
        <DialogContent title={t("backup.confirmTitle")} className="max-w-md">
          <div className="grid gap-4">
            <ul className="grid list-disc gap-1.5 ps-5 text-sm text-muted-foreground">
              <li>{t("backup.confirm1")}</li>
              <li>{t("backup.confirm2")}</li>
              <li>{t("backup.confirm3")}</li>
            </ul>
            <FormError error={restore.error} />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setAsking(false)}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" disabled={restore.isPending} onClick={() => restore.mutate()}>
                {restore.isPending ? t("backup.restoring") : t("backup.restoreNow")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={done !== null} onOpenChange={(open) => !open && void finish()}>
        <DialogContent title={t("backup.restoredTitle")} className="max-w-md">
          {done && (
            <div className="grid gap-4">
              <p className="text-sm text-muted-foreground">{t("backup.restoredBody", { when: formatDate(done.created_at, i18n.language, true) })}</p>
              <p className="break-all text-xs" dir="ltr">
                {t("backup.safety")}: <code>{done.safety}</code>
              </p>
              {done.warning && (
                <p className="text-sm text-warning" dir="auto">
                  {done.warning}
                </p>
              )}
              <div className="flex justify-end">
                <Button onClick={() => void finish()}>{t("backup.signInAgain")}</Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </Card>
  );
}

export function BackupPage() {
  const { t } = useTranslation();
  const view = useQuery({ queryKey: key, queryFn: () => api<BackupsView>("backups") });
  return (
    <>
      <PageHeader title={t("backup.title")} subtitle={t("backup.subtitle")} />
      <div className="grid gap-4">
        <FormError error={view.error} />
        {view.isPending || !view.data ? (
          <Skeleton className="h-48" />
        ) : (
          <>
            <MakeCard view={view.data} />
            <Files view={view.data} />
            {/* Keyed by what is stored, so the form starts from it again after a save. */}
            <Schedule key={JSON.stringify(view.data.settings)} view={view.data} />
            <Restore />
          </>
        )}
      </div>
    </>
  );
}
