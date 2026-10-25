import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleAlertIcon, CircleCheckIcon, SaveIcon, UndoIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { DnsTab } from "@/components/core/dns-tab";
import { JsonTab } from "@/components/core/json-tab";
import { OutboundsTab } from "@/components/core/outbounds-tab";
import { RoutingTab } from "@/components/core/routing-tab";
import { ZrayTab } from "@/components/core/zray-tab";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { api, post, put, type CoreView, type CoreWarning } from "@/lib/api";
import { canonical, type Json } from "@/lib/core-form";
import { useCan } from "@/lib/permissions";

const tabs = ["outbounds", "routing", "dns", "zray", "json"] as const;
type Tab = (typeof tabs)[number];

/** What the core made of the last check or apply. */
type Outcome = { kind: "checked" | "applied" | "proposed"; warnings: CoreWarning[]; how?: string };

function Warnings({ warnings }: { warnings: CoreWarning[] }) {
  const { t } = useTranslation();
  if (warnings.length === 0) return null;
  return (
    <div className="mt-3 rounded-md border border-warning/40 bg-warning/10 p-3 text-sm">
      <div className="mb-1 flex items-center gap-2 font-medium">
        <CircleAlertIcon className="size-4 text-warning" /> {t("core.warnings", { count: warnings.length })}
      </div>
      <ul className="grid gap-1 text-muted-foreground">
        {warnings.map((warning, index) => (
          <li key={index} dir="auto">
            <code className="text-xs" dir="ltr">
              {warning.path}
            </code>{" "}
            {warning.message}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function CorePage() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const canWrite = useCan("core.write");
  const core = useQuery({ queryKey: ["core"], queryFn: () => api<CoreView>("core"), refetchInterval: 15_000 });
  const [tab, setTab] = useState<Tab>("outbounds");
  // What is being edited; the stored settings stay as they are until applied.
  const [draft, setDraft] = useState<Json | null>(null);
  const [outcome, setOutcome] = useState<Outcome | null>(null);

  useEffect(() => {
    if (core.data && draft === null) setDraft(structuredClone(core.data.settings));
  }, [core.data, draft]);

  const stored = core.data?.settings;
  const dirty = useMemo(() => draft !== null && stored !== undefined && canonical(draft) !== canonical(stored), [draft, stored]);

  const check = useMutation({ meta: { quiet: true },
    mutationFn: () => post<{ warnings: CoreWarning[] }>("core/validate", { settings: draft }),
    onSuccess: (result) => setOutcome({ kind: "checked", warnings: result.warnings }),
  });
  const apply = useMutation({
    mutationFn: () => put<{ settings: Json; applied: string; warnings: CoreWarning[] }>("core", { settings: draft }),
    onSuccess: async (result) => {
      client.setQueryData<CoreView>(["core"], (view) => (view ? { ...view, settings: result.settings } : view));
      setDraft(structuredClone(result.settings));
      setOutcome({ kind: "applied", warnings: result.warnings, how: result.applied });
      await client.invalidateQueries({ queryKey: ["core"] });
      await client.invalidateQueries({ queryKey: ["health"] });
    },
  });
  const failure = check.error ?? apply.error;

  const change = useCallback(
    (apply_: (draft: Json) => void) => {
      setOutcome(null);
      check.reset();
      apply.reset();
      setDraft((current) => {
        const next = structuredClone(current ?? {});
        apply_(next);
        return next;
      });
    },
    // The resets do not change between renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  /** The editor holds `settings` instead of what it held. */
  const replaceAll = (settings: Json) =>
    change((next) => {
      for (const key of Object.keys(next)) delete next[key];
      Object.assign(next, settings);
    });

  if (core.isPending || draft === null || !core.data) {
    return (
      <>
        <PageHeader title={t("core.title")} subtitle={t("core.subtitle")} />
        <FormError error={core.error} />
        <Skeleton className="h-64" />
      </>
    );
  }
  const view = core.data;
  const props = { settings: draft, change, core: view };

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <PageHeader title={t("core.title")} subtitle={t("core.subtitle")} />
        <Badge variant={view.running ? "success" : "secondary"} className="text-sm">
          {view.running ? t("dashboard.running") : t("dashboard.stopped")}
        </Badge>
      </div>

      <Segmented
        label={t("core.sections")}
        className="mb-4"
        value={tab}
        onChange={setTab}
        options={tabs.map((value) => ({ value, label: t(`core.tab_${value}`) }))}
      />

      {/* Someone who may only look gets the editor with every control off. */}
      <fieldset disabled={!canWrite} className="m-0 min-w-0 border-0 p-0">
        {tab === "outbounds" && <OutboundsTab {...props} />}
        {tab === "routing" && <RoutingTab {...props} />}
        {tab === "dns" && <DnsTab {...props} />}
        {tab === "zray" && (
          <ZrayTab
            {...props}
            onPropose={(proposed, warnings) => {
              replaceAll(proposed);
              setOutcome({ kind: "proposed", warnings });
              setTab("outbounds");
            }}
          />
        )}
        {tab === "json" && <JsonTab settings={draft} core={view} replace={replaceAll} />}
      </fieldset>

      <Card className={dirty || outcome ? "sticky bottom-4 z-20 mt-4 shadow-lg" : "mt-4"}>
        <CardContent className="p-4">
          {dirty ? (
            <div className="flex flex-wrap items-center justify-between gap-3">
              <span className="flex items-center gap-2 text-sm">
                <span className="size-2 rounded-full bg-warning" aria-hidden />
                {outcome?.kind === "proposed" ? t("core.proposedBar") : t("core.unsaved")}
              </span>
              <div className="flex gap-2">
                <Button variant="ghost" size="sm" onClick={() => { setDraft(structuredClone(view.settings)); setOutcome(null); }}>
                  <UndoIcon /> {t("core.discard")}
                </Button>
                <Button variant="outline" size="sm" disabled={check.isPending} onClick={() => check.mutate()}>
                  {t("core.check")}
                </Button>
                <Button size="sm" disabled={apply.isPending} onClick={() => apply.mutate()}>
                  <SaveIcon /> {apply.isPending ? t("core.applying") : t("core.apply")}
                </Button>
              </div>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{t("core.nothingToApply")}</p>
          )}
          <FormError error={failure} />
          {outcome && outcome.kind !== "proposed" && (
            <p className="mt-3 flex items-center gap-2 text-sm text-success">
              <CircleCheckIcon className="size-4" />
              {outcome.kind === "applied" ? t(`core.applied_${outcome.how ?? "reloaded"}`) : t("core.checkedOk")}
            </p>
          )}
          {outcome && <Warnings warnings={outcome.warnings} />}
        </CardContent>
      </Card>
    </>
  );
}
