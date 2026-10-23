import { useMutation } from "@tanstack/react-query";
import { CircleCheckIcon, DatabaseIcon, FileJsonIcon, LinkIcon, TriangleAlertIcon, UploadIcon } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { SelectField } from "@/components/ui/select";
import { SwitchField } from "@/components/ui/switch";
import {
  del,
  post,
  upload,
  type MigrateAnalysis,
  type MigrateKind,
  type MigrateOptions,
  type MigrateReport,
  type MigrateWarning,
} from "@/lib/api";
import { defaultOptions, groupWarnings, looksLikeDatabase, namesOf } from "@/lib/migrate";
import { formatBytes } from "@/lib/utils";

const kinds: MigrateKind[] = ["3x-ui", "pasarguard", "marzban"];

/** One kind of trouble, with what it is about. */
function WarningList({ warnings, total }: { warnings: MigrateWarning[]; total: number }) {
  const { t, i18n } = useTranslation();
  const groups = groupWarnings(warnings);
  if (groups.length === 0) return null;
  return (
    <div className="grid gap-2">
      <h3 className="flex items-center gap-2 text-sm font-medium">
        <TriangleAlertIcon className="size-4 text-warning" /> {t("migrate.warnings", { count: total, formatted: new Intl.NumberFormat(i18n.language).format(total) })}
      </h3>
      <ul className="grid gap-2">
        {groups.map((group) => {
          const names = namesOf(group);
          const first = group.items[0];
          return (
            <li key={`${group.scope}/${group.code}`} className="rounded-md border p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{t(`migrate.scope.${group.scope}`, { defaultValue: group.scope })}</Badge>
                {group.items.length > 1 && (
                  <span className="tabular text-xs text-muted-foreground">× {new Intl.NumberFormat(i18n.language).format(group.items.length)}</span>
                )}
              </div>
              <p className="mt-1.5">
                {t(`migrate.warning.${group.code}`, { name: first.name, defaultValue: first.detail })}
              </p>
              {names.shown.length > 0 && group.items.length > 1 && (
                <p className="mt-1 break-all text-xs text-muted-foreground" dir="ltr">
                  {names.shown.join(", ")}
                  {names.more > 0 && ` … +${names.more}`}
                </p>
              )}
            </li>
          );
        })}
      </ul>
      {total > warnings.length && <p className="text-xs text-muted-foreground">{t("migrate.warningsCapped", { shown: warnings.length, total })}</p>}
    </div>
  );
}

function Count({ label, value }: { label: string; value: number }) {
  const { i18n } = useTranslation();
  return (
    <div className="rounded-md border p-3">
      <div className="tabular text-2xl font-semibold">{new Intl.NumberFormat(i18n.language).format(value)}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

function Pick({ analysis, onChange }: { analysis: MigrateAnalysis; onChange: (next: MigrateAnalysis) => void }) {
  const { t } = useTranslation();
  const input = useRef<HTMLInputElement>(null);
  const give = useMutation({
    mutationFn: (file: File) => upload<MigrateAnalysis>(`migrate/${analysis.id}/xray-config`, file, {}, "PUT"),
    onSuccess: onChange,
  });
  return (
    <div className="grid gap-2 rounded-md border border-warning/40 p-3">
      <p className="text-sm">
        {t("migrate.needsConfig")}
      </p>
      <input
        ref={input}
        type="file"
        accept=".json,application/json"
        className="sr-only"
        aria-label={t("migrate.pickConfig")}
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) give.mutate(file);
          e.target.value = "";
        }}
      />
      <div>
        <Button type="button" variant="outline" disabled={give.isPending} onClick={() => input.current?.click()}>
          <FileJsonIcon /> {t("migrate.pickConfig")}
        </Button>
      </div>
      <FormError error={give.error} />
    </div>
  );
}

function Upload({ onAnalysed }: { onAnalysed: (analysis: MigrateAnalysis) => void }) {
  const { t, i18n } = useTranslation();
  const input = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [kind, setKind] = useState("");
  const send = useMutation({
    mutationFn: () => {
      if (!file) throw new Error("no file");
      return upload<MigrateAnalysis>(`migrate${kind ? `?kind=${encodeURIComponent(kind)}` : ""}`, file);
    },
    onSuccess: onAnalysed,
  });
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("migrate.uploadTitle")}</CardTitle>
        <CardDescription>{t("migrate.uploadBody")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        <ul className="grid gap-1.5 text-sm text-muted-foreground">
          {kinds.map((name) => (
            <li key={name} className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-medium text-foreground">{t(`migrate.kind.${name}`)}</span>
              <code className="text-xs" dir="ltr">
                {t(`migrate.where.${name}`)}
              </code>
            </li>
          ))}
        </ul>
        <input
          ref={input}
          type="file"
          className="sr-only"
          aria-label={t("migrate.pick")}
          onChange={(e) => {
            setFile(e.target.files?.[0] ?? null);
            send.reset();
          }}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button type="button" variant="outline" onClick={() => input.current?.click()}>
            <UploadIcon /> {t("migrate.pick")}
          </Button>
          {file && (
            <span className="min-w-0 break-all text-sm" dir="ltr">
              {file.name} · {formatBytes(file.size, i18n.language)}
            </span>
          )}
        </div>
        {file && !looksLikeDatabase(file.name) && <p className="text-sm text-warning">{t("migrate.notADatabase")}</p>}
        <SelectField
          label={t("migrate.kindLabel")}
          hint={t("migrate.kindHint")}
          value={kind}
          onChange={(e) => setKind(e.target.value)}
          options={[{ value: "", label: t("migrate.kindAuto") }, ...kinds.map((name) => ({ value: name, label: t(`migrate.kind.${name}`) }))]}
        />
        <FormError error={send.error} />
        <Button className="justify-self-start" disabled={!file || send.isPending} onClick={() => send.mutate()}>
          {send.isPending ? t("migrate.reading") : t("migrate.read")}
        </Button>
      </CardContent>
    </Card>
  );
}

function Preview({
  analysis,
  onChange,
  onDone,
  onDiscard,
}: {
  analysis: MigrateAnalysis;
  onChange: (next: MigrateAnalysis) => void;
  onDone: (report: MigrateReport) => void;
  onDiscard: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [options, setOptions] = useState<MigrateOptions>(defaultOptions);
  const [asking, setAsking] = useState(false);
  const set = (patch: Partial<MigrateOptions>) => setOptions((current) => ({ ...current, ...patch }));
  const run = useMutation({
    mutationFn: () => post<MigrateReport>(`migrate/${analysis.id}/apply`, options),
    onSuccess: (report) => {
      setAsking(false);
      onDone(report);
    },
  });
  const discard = useMutation({ mutationFn: () => del(`migrate/${analysis.id}`), onSuccess: onDiscard });
  const s = analysis.summary;
  return (
    <Card>
      <CardHeader className="flex-row flex-wrap items-start justify-between gap-3 space-y-0">
        <div>
          <CardTitle className="flex flex-wrap items-center gap-2">
            <DatabaseIcon className="size-4 text-primary" /> {t("migrate.previewTitle")}
            <Badge>{t(`migrate.kind.${analysis.kind}`)}</Badge>
          </CardTitle>
          <CardDescription className="mt-1">{t("migrate.previewBody", { size: formatBytes(analysis.bytes, i18n.language) })}</CardDescription>
        </div>
        <Button variant="ghost" disabled={discard.isPending} onClick={() => discard.mutate()}>
          {t("migrate.discard")}
        </Button>
      </CardHeader>
      <CardContent className="grid gap-5">
        {analysis.needs_xray_config && <Pick analysis={analysis} onChange={onChange} />}
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          <Count label={t("migrate.counts.inbounds")} value={s.inbounds} />
          <Count label={t("migrate.counts.users")} value={s.users} />
          <Count label={t("migrate.counts.groups")} value={s.groups} />
          <Count label={t("migrate.counts.templates")} value={s.templates} />
          <Count label={t("migrate.counts.hosts")} value={s.hosts} />
          <Count label={t("migrate.counts.usage")} value={s.usage_hours} />
        </div>
        <p className="flex items-start gap-2 text-sm">
          <LinkIcon className="mt-0.5 size-4 shrink-0 text-primary" /> {t(`migrate.links.${s.links}`)}
        </p>
        {s.inbound_names.length > 0 && (
          <p className="break-all text-xs text-muted-foreground" dir="ltr">
            {s.inbound_names.join(", ")}
            {s.inbounds > s.inbound_names.length && ` … +${s.inbounds - s.inbound_names.length}`}
          </p>
        )}
        <WarningList warnings={analysis.warnings} total={analysis.warnings_total} />

        <div className="grid gap-4 border-t pt-4">
          <h3 className="text-sm font-medium">{t("migrate.optionsTitle")}</h3>
          <div className="grid gap-4 sm:grid-cols-2">
            <SwitchField label={t("migrate.opt.inbounds")} checked={options.inbounds} onCheckedChange={(inbounds) => set({ inbounds })} />
            <SwitchField label={t("migrate.opt.users")} checked={options.users} onCheckedChange={(users) => set({ users })} />
            <SwitchField label={t("migrate.opt.hosts")} checked={options.hosts && options.inbounds} disabled={!options.inbounds} onCheckedChange={(hosts) => set({ hosts })} />
            <SwitchField label={t("migrate.opt.templates")} checked={options.templates} onCheckedChange={(templates) => set({ templates })} />
            <SwitchField label={t("migrate.opt.usage")} checked={options.usage && options.users} disabled={!options.users} onCheckedChange={(usage) => set({ usage })} />
            <SwitchField label={t("migrate.opt.settings")} hint={t("migrate.opt.settingsHint")} checked={options.settings} onCheckedChange={(settings) => set({ settings })} />
          </div>
          <SwitchField
            label={t("migrate.opt.enable")}
            hint={t("migrate.opt.enableHint")}
            checked={options.enable_inbounds}
            disabled={!options.inbounds}
            onCheckedChange={(enable_inbounds) => set({ enable_inbounds })}
          />
          <SelectField
            label={t("migrate.opt.conflict")}
            hint={t("migrate.opt.conflictHint")}
            value={options.on_conflict}
            onChange={(e) => set({ on_conflict: e.target.value === "rename" ? "rename" : "skip" })}
            options={[
              { value: "skip", label: t("migrate.opt.skip") },
              { value: "rename", label: t("migrate.opt.rename") },
            ]}
          />
        </div>
        <FormError error={discard.error} />
        <Button className="justify-self-start" disabled={run.isPending} onClick={() => setAsking(true)}>
          {t("migrate.import")}
        </Button>
      </CardContent>

      <Dialog open={asking} onOpenChange={setAsking}>
        <DialogContent title={t("migrate.confirmTitle")} className="max-w-md">
          <div className="grid gap-4">
            <ul className="grid list-disc gap-1.5 ps-5 text-sm text-muted-foreground">
              <li>{t("migrate.confirm1")}</li>
              <li>{t("migrate.confirm2")}</li>
              <li>{t("migrate.confirm3")}</li>
            </ul>
            <FormError error={run.error} />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setAsking(false)}>
                {t("common.cancel")}
              </Button>
              <Button disabled={run.isPending} onClick={() => run.mutate()}>
                {run.isPending ? t("migrate.importing") : t("migrate.importNow")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </Card>
  );
}

function Report({ report, onAgain }: { report: MigrateReport; onAgain: () => void }) {
  const { t } = useTranslation();
  const n = report.imported;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <CircleCheckIcon className="size-5 text-success" /> {t("migrate.doneTitle")}
        </CardTitle>
        <CardDescription>{t("migrate.doneBody")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-5">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          <Count label={t("migrate.counts.inbounds")} value={n.inbounds} />
          <Count label={t("migrate.counts.users")} value={n.users} />
          <Count label={t("migrate.counts.groups")} value={n.groups} />
          <Count label={t("migrate.counts.templates")} value={n.templates} />
          <Count label={t("migrate.counts.hosts")} value={n.hosts} />
          <Count label={t("migrate.counts.certificates")} value={n.certificates} />
        </div>
        <p className="break-all text-xs" dir="ltr">
          {t("migrate.backupKept")}: <code>{report.backup}</code>
        </p>
        <p className="flex items-start gap-2 text-sm">
          <LinkIcon className="mt-0.5 size-4 shrink-0 text-primary" /> {t(`migrate.links.${report.links}`)}
        </p>
        {report.core_error && (
          <p className="text-sm text-warning">
            {t("migrate.coreError", { message: report.core_error })}
          </p>
        )}
        <WarningList warnings={report.warnings} total={report.warnings_total} />
        <div className="grid gap-2 rounded-md border p-3 text-sm">
          <h3 className="font-medium">{t("migrate.nextTitle")}</h3>
          <ol className="grid list-decimal gap-1 ps-5 text-muted-foreground">
            <li>{t("migrate.next1")}</li>
            <li>{t("migrate.next2")}</li>
            <li>{t("migrate.next3")}</li>
          </ol>
        </div>
        <Button variant="outline" className="justify-self-start" onClick={onAgain}>
          {t("migrate.another")}
        </Button>
      </CardContent>
    </Card>
  );
}

export function MigratePage() {
  const { t } = useTranslation();
  const [analysis, setAnalysis] = useState<MigrateAnalysis | null>(null);
  const [report, setReport] = useState<MigrateReport | null>(null);
  return (
    <>
      <PageHeader title={t("migrate.title")} subtitle={t("migrate.subtitle")} />
      <div className="grid gap-4">
        {report ? (
          <Report
            report={report}
            onAgain={() => {
              setReport(null);
              setAnalysis(null);
            }}
          />
        ) : analysis ? (
          // Keyed by the upload, so the options start again for the next one.
          <Preview
            key={analysis.id}
            analysis={analysis}
            onChange={setAnalysis}
            onDone={(done) => {
              setReport(done);
              setAnalysis(null);
            }}
            onDiscard={() => setAnalysis(null)}
          />
        ) : (
          <Upload onAnalysed={setAnalysis} />
        )}
      </div>
    </>
  );
}
