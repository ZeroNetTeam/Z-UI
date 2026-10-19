import { DownloadIcon, PauseIcon, PlayIcon, SearchIcon, Trash2Icon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/layout/layout";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import type { LogLevel, LogRecord } from "@/lib/api";
import { cn } from "@/lib/utils";

/** How many lines the page keeps; older ones scroll away for good. */
const KEEP = 2000;

const sources = ["all", "core", "panel"] as const;
const levels: LogLevel[] = ["error", "warn", "info", "debug"];

const levelTone: Record<LogLevel, string> = {
  error: "text-destructive",
  warn: "text-warning",
  info: "text-foreground",
  debug: "text-muted-foreground",
  trace: "text-muted-foreground",
};

/** `HH:MM:SS` in the reader's clock. */
function clock(time: number, locale: string) {
  return new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date(time));
}

type Connection = "connecting" | "live" | "lost";

/** The panel's log, and the core's: what happened lately, then what happens. */
export function LogsPage() {
  const { t } = useTranslation();
  const [source, setSource] = useState<(typeof sources)[number]>("all");
  const [level, setLevel] = useState<string>("");
  const [search, setSearch] = useState("");
  const [lines, setLines] = useState<LogRecord[]>([]);
  const [paused, setPaused] = useState(false);
  const [connection, setConnection] = useState<Connection>("connecting");
  const pausedRef = useRef(false);
  const held = useRef<LogRecord[]>([]);
  const listRef = useRef<HTMLDivElement>(null);
  const stuck = useRef(true);
  const [settledSearch, setSettledSearch] = useState("");

  useEffect(() => {
    const timer = setTimeout(() => setSettledSearch(search.trim()), 300);
    return () => clearTimeout(timer);
  }, [search]);

  // One stream per set of filters; changing a filter starts a fresh one.
  useEffect(() => {
    setLines([]);
    held.current = [];
    setConnection("connecting");
    let lastId = 0;
    const params = new URLSearchParams({ limit: "300" });
    if (source !== "all") params.set("source", source);
    if (level) params.set("level", level);
    if (settledSearch) params.set("search", settledSearch);
    const stream = new EventSource(new URL(`api/logs/stream?${params}`, document.baseURI));
    stream.onopen = () => setConnection("live");
    stream.onerror = () => setConnection("lost");
    stream.onmessage = (message) => {
      let record: LogRecord;
      try {
        record = JSON.parse(message.data) as LogRecord;
      } catch {
        return;
      }
      // A reconnect replays the backlog; what was seen is not shown twice.
      if (record.id <= lastId) return;
      lastId = record.id;
      if (pausedRef.current) {
        held.current.push(record);
        return;
      }
      setLines((current) => [...current, record].slice(-KEEP));
    };
    return () => stream.close();
  }, [source, level, settledSearch]);

  const setPause = useCallback((value: boolean) => {
    pausedRef.current = value;
    setPaused(value);
    if (!value && held.current.length > 0) {
      const waiting = held.current;
      held.current = [];
      setLines((current) => [...current, ...waiting].slice(-KEEP));
    }
  }, []);

  // Follow the end of the log, unless the reader has scrolled up to read.
  useEffect(() => {
    const list = listRef.current;
    if (list && stuck.current) list.scrollTop = list.scrollHeight;
  }, [lines]);

  const text = useMemo(
    () =>
      lines
        .map((l) => `${new Date(l.time).toISOString()} ${l.level.toUpperCase().padEnd(5)} ${l.target} ${l.message}`)
        .join("\n"),
    [lines],
  );
  const download = () => {
    const url = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `z-ui-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.log`;
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <>
      <PageHeader title={t("logs.title")} subtitle={t("logs.subtitle")} />
      <div className="mb-3 flex flex-col gap-2 lg:flex-row lg:items-center">
        <div className="flex rounded-lg border p-1 text-sm" role="radiogroup" aria-label={t("logs.source")}>
          {sources.map((item) => (
            <button
              key={item}
              type="button"
              role="radio"
              aria-checked={item === source}
              onClick={() => setSource(item)}
              className={cn(
                "rounded-md px-3 py-1 transition-colors",
                item === source ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent",
              )}
            >
              {t(`logs.source_${item}`)}
            </button>
          ))}
        </div>
        <Select value={level} onChange={(e) => setLevel(e.target.value)} aria-label={t("logs.level")} className="lg:w-44">
          <option value="">{t("logs.allLevels")}</option>
          {levels.map((item) => (
            <option key={item} value={item}>
              {t(`logs.level_${item}`)}
            </option>
          ))}
        </Select>
        <div className="relative flex-1">
          <SearchIcon className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("logs.search")} aria-label={t("logs.search")} className="ps-9" />
        </div>
        <div className="flex items-center gap-1">
          <Button variant="outline" size="sm" aria-pressed={paused} onClick={() => setPause(!paused)}>
            {paused ? <PlayIcon /> : <PauseIcon />}
            {paused ? t("logs.resume") : t("logs.pause")}
          </Button>
          <Button variant="outline" size="icon" aria-label={t("logs.clear")} onClick={() => setLines([])}>
            <Trash2Icon />
          </Button>
          <Button variant="outline" size="icon" aria-label={t("logs.download")} onClick={download} disabled={lines.length === 0}>
            <DownloadIcon />
          </Button>
        </div>
      </div>

      <Card className="overflow-hidden">
        <div className="flex items-center justify-between border-b px-4 py-2 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span
              className={cn(
                "size-1.5 rounded-full",
                connection === "live" && !paused && "animate-pulse bg-success",
                connection === "live" && paused && "bg-warning",
                connection === "connecting" && "bg-muted-foreground",
                connection === "lost" && "bg-destructive",
              )}
              aria-hidden
            />
            {connection === "lost" ? t("logs.lost") : paused ? t("logs.paused", { count: held.current.length }) : connection === "live" ? t("logs.live") : t("logs.connecting")}
          </span>
          <span className="tabular">{t("logs.count", { count: lines.length })}</span>
        </div>
        <div
          ref={listRef}
          onScroll={(e) => {
            const el = e.currentTarget;
            stuck.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          }}
          className="h-[60svh] min-h-72 overflow-auto bg-background/40 p-2 font-mono text-xs leading-5"
          dir="ltr"
          role="log"
          aria-live="off"
          aria-label={t("logs.title")}
        >
          {lines.length === 0 ? (
            <p className="p-4 font-sans text-sm text-muted-foreground" dir="auto">
              {t("logs.empty")}
            </p>
          ) : (
            lines.map((line) => (
              <div key={line.id} className="flex gap-3 rounded px-2 hover:bg-accent/50">
                <span className="tabular shrink-0 text-muted-foreground">{clock(line.time, "en")}</span>
                <span className={cn("w-12 shrink-0 font-semibold uppercase", levelTone[line.level])}>{line.level}</span>
                <span className="w-14 shrink-0 text-muted-foreground">{line.source}</span>
                <span className="min-w-0 break-words whitespace-pre-wrap" title={line.target}>
                  {line.message}
                </span>
              </div>
            ))
          )}
        </div>
      </Card>
    </>
  );
}
