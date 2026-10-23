import { useInfiniteQuery } from "@tanstack/react-query";
import { HistoryIcon, RefreshCwIcon, SearchIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { PageHeader } from "@/components/layout/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SwitchField } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api, type AuditEntry, type AuditPage } from "@/lib/api";
import { formatDate } from "@/lib/users";
import { cn } from "@/lib/utils";

/** The kinds of action the filter offers: the part of the name before the dot. */
const kinds = ["user", "group", "template", "inbound", "host", "certificate", "core", "settings", "logs", "admin", "role", "api_key", "auth"];

const PAGE = 50;

/** `value`, once it has stopped changing for a moment. */
function useSettled<T>(value: T, ms = 300): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

/** Green for what worked, amber for what was refused, red for what broke. */
function statusVariant(status: number): "success" | "default" | "destructive" {
  if (status < 400) return "success";
  return status === 401 || status === 403 || status === 429 ? "default" : "destructive";
}

export function AuditPage() {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState("");
  const [failed, setFailed] = useState(false);
  const query = useSettled(search.trim());

  const log = useInfiniteQuery({
    queryKey: ["audit", query, kind, failed],
    queryFn: ({ pageParam }) => {
      const params = new URLSearchParams({ limit: String(PAGE) });
      if (query) params.set("q", query);
      if (kind) params.set("action", `${kind}.`);
      if (failed) params.set("failed", "true");
      if (pageParam) params.set("before", String(pageParam));
      return api<AuditPage>(`audit?${params}`);
    },
    initialPageParam: 0,
    getNextPageParam: (last) => last.next ?? undefined,
  });
  const rows: AuditEntry[] = log.data?.pages.flatMap((page) => page.entries) ?? [];
  const action = (entry: AuditEntry) => t(`audit.action.${entry.action}`, { defaultValue: entry.action });

  return (
    <>
      <PageHeader title={t("audit.title")} subtitle={t("audit.subtitle", { days: 180 })} />

      <div className="mb-4 grid gap-2 sm:grid-cols-[1fr_auto_auto]">
        <div className="relative">
          <SearchIcon className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("audit.search")} className="ps-9" aria-label={t("audit.search")} />
        </div>
        <Select value={kind} onChange={(e) => setKind(e.target.value)} aria-label={t("audit.kind")} className="sm:w-44">
          <option value="">{t("audit.anyKind")}</option>
          {kinds.map((value) => (
            <option key={value} value={value}>
              {t(`audit.kinds.${value}`)}
            </option>
          ))}
        </Select>
        <Button variant="outline" size="icon" aria-label={t("users.refresh")} onClick={() => void log.refetch()}>
          <RefreshCwIcon className={cn(log.isFetching && "animate-spin")} />
        </Button>
      </div>
      <div className="mb-4 max-w-md">
        <SwitchField label={t("audit.failedOnly")} hint={t("audit.failedOnlyHint")} checked={failed} onCheckedChange={setFailed} />
      </div>
      <div className="mb-4">
        <FormError error={log.error} />
      </div>

      <Card>
        <CardContent className="p-0">
          {log.isPending ? (
            <div className="grid gap-2 p-5">
              <Skeleton className="h-10" />
              <Skeleton className="h-10" />
              <Skeleton className="h-10" />
            </div>
          ) : rows.length === 0 ? (
            <div className="flex flex-col items-center gap-3 p-10 text-center text-sm text-muted-foreground">
              <HistoryIcon className="size-8 text-primary/60" />
              {query || kind || failed ? t("audit.noMatch") : t("audit.empty")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("audit.when")}</TableHead>
                  <TableHead>{t("audit.who")}</TableHead>
                  <TableHead>{t("audit.what")}</TableHead>
                  <TableHead className="hidden lg:table-cell">{t("audit.from")}</TableHead>
                  <TableHead className="w-16">{t("audit.result")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((entry) => (
                  <TableRow key={entry.id}>
                    <TableCell className="tabular text-xs whitespace-nowrap text-muted-foreground">{formatDate(entry.at, i18n.language, true)}</TableCell>
                    <TableCell>
                      <div className="font-medium" dir="ltr">
                        {entry.admin}
                      </div>
                      {entry.via !== "session" && (
                        <Badge variant="outline" className="mt-0.5" dir="ltr">
                          {entry.via}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="text-sm">{action(entry)}</div>
                      {(entry.target || entry.detail) && (
                        <div className="max-w-md truncate text-xs text-muted-foreground" dir="auto" title={`${entry.target} ${entry.detail}`}>
                          {entry.target}
                          {entry.target && entry.detail ? " · " : ""}
                          {entry.detail}
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="hidden font-mono text-xs text-muted-foreground lg:table-cell" dir="ltr">
                      {entry.ip}
                    </TableCell>
                    <TableCell>
                      <Badge variant={statusVariant(entry.status)} className="tabular">
                        {entry.status}
                      </Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {log.hasNextPage && (
        <div className="mt-4 flex justify-center">
          <Button variant="outline" disabled={log.isFetchingNextPage} onClick={() => void log.fetchNextPage()}>
            {t("audit.more")}
          </Button>
        </div>
      )}
    </>
  );
}
