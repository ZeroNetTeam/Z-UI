import { useQuery } from "@tanstack/react-query";
import { DownloadIcon, LockIcon, PlayIcon, SearchIcon } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "@/components/copy-button";
import { FormError } from "@/components/form-error";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { curl, example, operations, rows, url, type OpenApiDoc, type Operation } from "@/lib/openapi";
import { cn } from "@/lib/utils";

const methodStyle: Record<string, "success" | "default" | "secondary" | "destructive"> = {
  get: "success",
  post: "default",
  put: "secondary",
  patch: "secondary",
  delete: "destructive",
};

function MethodBadge({ method }: { method: string }) {
  return (
    <Badge variant={methodStyle[method] ?? "outline"} className="w-14 justify-center font-mono uppercase" dir="ltr">
      {method}
    </Badge>
  );
}

/** Text with `code` and **bold** in it, as the document writes descriptions. */
function Prose({ text }: { text: string }) {
  return (
    <div className="grid gap-2 text-sm text-muted-foreground">
      {text
        .split(/\n\n+/)
        .filter(Boolean)
        .map((paragraph, index) => (
          <p key={index} dir="auto">
            {paragraph.split(/(\*\*[^*]+\*\*|`[^`]+`)/).map((part, at) =>
              part.startsWith("**") ? (
                <strong key={at} className="font-semibold text-foreground">
                  {part.slice(2, -2)}
                </strong>
              ) : part.startsWith("`") ? (
                <code key={at} className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground" dir="ltr">
                  {part.slice(1, -1)}
                </code>
              ) : (
                <Fragment key={at}>{part}</Fragment>
              ),
            )}
          </p>
        ))}
    </div>
  );
}

/** Fields of a request or response body, nested ones indented. */
function Fields({ doc, schema }: { doc: OpenApiDoc; schema: Parameters<typeof rows>[1] }) {
  const { t } = useTranslation();
  const list = rows(doc, schema);
  if (list.length === 0) return null;
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-xs">
        <thead className="bg-muted/40 text-muted-foreground">
          <tr>
            <th className="px-3 py-1.5 text-start font-medium">{t("developer.field")}</th>
            <th className="px-3 py-1.5 text-start font-medium">{t("developer.type")}</th>
            <th className="px-3 py-1.5 text-start font-medium">{t("developer.description")}</th>
          </tr>
        </thead>
        <tbody>
          {list.map((row, index) => (
            <tr key={index} className="border-t align-top">
              <td className="px-3 py-1.5 font-mono whitespace-nowrap" dir="ltr" style={{ paddingInlineStart: `${0.75 + row.depth * 1}rem` }}>
                {row.name}
                {row.required && <span className="text-destructive"> *</span>}
              </td>
              <td className="px-3 py-1.5 font-mono text-muted-foreground" dir="ltr">
                {row.type}
              </td>
              <td className="px-3 py-1.5 text-muted-foreground" dir="auto">
                {row.description}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface Outcome {
  status: number;
  statusText: string;
  ms: number;
  body: string;
}

/** The call, made for real from the browser, with the session or a key. */
function TryIt({ doc, op }: { doc: OpenApiDoc; op: Operation }) {
  const { t } = useTranslation();
  const initial = useMemo(() => (op.body ? JSON.stringify(example(doc, op.body), null, 2) : ""), [doc, op]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [body, setBody] = useState(initial);
  const [auth, setAuth] = useState<"session" | "key">("session");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    setValues({});
    setBody(initial);
    setOutcome(null);
    setFailure(null);
  }, [initial, op]);

  const stream = op.responses.some((response) => response.contentType === "text/event-stream");
  const path = url(op, values, op.parameters);
  const address = new URL(`api${path}`, document.baseURI).href;
  const withBody = op.body !== null;

  const send = async () => {
    setBusy(true);
    setOutcome(null);
    setFailure(null);
    const started = performance.now();
    try {
      const headers: Record<string, string> = { Accept: "application/json" };
      if (withBody) headers["Content-Type"] = "application/json";
      if (auth === "key" && key) headers.Authorization = `Bearer ${key}`;
      const response = await fetch(address, {
        method: op.method.toUpperCase(),
        headers,
        body: withBody ? body : undefined,
        credentials: auth === "key" ? "omit" : "same-origin",
      });
      const text = await response.text();
      let shown = text;
      try {
        shown = JSON.stringify(JSON.parse(text), null, 2);
      } catch {
        // Not JSON: show it as it came.
      }
      setOutcome({
        status: response.status,
        statusText: response.statusText,
        ms: Math.round(performance.now() - started),
        body: shown.length > 20_000 ? `${shown.slice(0, 20_000)}\n…` : shown,
      });
    } catch (error) {
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-3 rounded-lg border bg-muted/20 p-4">
      <h4 className="text-sm font-semibold">{t("developer.tryIt")}</h4>
      {op.method !== "get" && <p className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs">{t("developer.realCall")}</p>}
      {stream ? (
        <p className="text-xs text-muted-foreground">{t("developer.streamNote")}</p>
      ) : (
        <>
          {op.parameters.length > 0 && (
            <div className="grid gap-2 sm:grid-cols-2">
              {op.parameters.map((parameter) => (
                <div key={`${parameter.in}:${parameter.name}`} className="grid gap-1">
                  <Label className="font-mono text-xs" dir="ltr">
                    {parameter.name}
                    {parameter.required && <span className="text-destructive"> *</span>}
                    <span className="ms-1.5 text-muted-foreground">({parameter.in})</span>
                  </Label>
                  <Input
                    value={values[parameter.name] ?? ""}
                    onChange={(e) => setValues((current) => ({ ...current, [parameter.name]: e.target.value }))}
                    dir="ltr"
                    className="font-mono text-xs"
                    placeholder={parameter.description}
                  />
                </div>
              ))}
            </div>
          )}
          {withBody && (
            <div className="grid gap-1">
              <Label className="text-xs">{t("developer.body")}</Label>
              <Textarea value={body} onChange={(e) => setBody(e.target.value)} dir="ltr" spellCheck={false} className="min-h-40 font-mono text-xs" />
            </div>
          )}
          <div className="flex flex-wrap items-center gap-3">
            <Segmented
              label={t("developer.authWith")}
              value={auth}
              onChange={setAuth}
              options={[
                { value: "session", label: t("developer.authSession") },
                { value: "key", label: t("developer.authKey") },
              ]}
            />
            {auth === "key" && (
              <Input
                type="password"
                value={key}
                onChange={(e) => setKey(e.target.value)}
                dir="ltr"
                placeholder="zk_…"
                className="w-64 font-mono text-xs"
                autoComplete="off"
                aria-label={t("developer.authKey")}
              />
            )}
            <Button onClick={() => void send()} disabled={busy}>
              <PlayIcon /> {t("developer.send")}
            </Button>
          </div>
          <div className="grid gap-1">
            <div className="flex items-center justify-between gap-2">
              <Label className="text-xs">cURL</Label>
              <CopyButton text={curl(op.method, address, withBody ? body.replace(/\s*\n\s*/g, " ") : null, true)} label={t("developer.copy")} />
            </div>
            <pre className="overflow-x-auto rounded-md border bg-background p-3 font-mono text-xs" dir="ltr">
              {curl(op.method, address, withBody ? body.replace(/\s*\n\s*/g, " ") : null, true)}
            </pre>
          </div>
          {failure && (
            <div role="alert" className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {failure}
            </div>
          )}
          {outcome && (
            <div className="grid gap-1">
              <div className="flex items-center gap-2 text-xs">
                <Badge variant={outcome.status < 400 ? "success" : "destructive"} className="tabular">
                  {outcome.status}
                </Badge>
                <span className="text-muted-foreground">{outcome.statusText}</span>
                <span className="tabular ms-auto text-muted-foreground">{outcome.ms} ms</span>
              </div>
              <pre className="max-h-96 overflow-auto rounded-md border bg-background p-3 font-mono text-xs" dir="ltr">
                {outcome.body || t("developer.noBody")}
              </pre>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function Detail({ doc, op }: { doc: OpenApiDoc; op: Operation }) {
  const { t } = useTranslation();
  // The generated "Needs" line is shown as a badge, so it is not said twice.
  const description = op.description.replace(/^\*\*Needs:\*\*[^\n]*\n*/, "");
  return (
    <div className="grid gap-5">
      <div className="grid gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <MethodBadge method={op.method} />
          <code className="font-mono text-sm break-all" dir="ltr">
            {op.path}
          </code>
        </div>
        <h3 className="text-lg font-semibold" dir="auto">
          {op.summary}
        </h3>
        <div className="flex flex-wrap gap-2">
          {op.open ? (
            <Badge variant="outline">{t("developer.openCall")}</Badge>
          ) : (
            <Badge variant="outline">
              <LockIcon /> {t("developer.needs")}: <span dir="ltr">{op.permission ?? t("developer.anySignIn")}</span>
            </Badge>
          )}
          <Badge variant="outline" className="font-mono" dir="ltr">
            {op.id}
          </Badge>
        </div>
        {description && <Prose text={description} />}
      </div>

      {op.parameters.length > 0 && (
        <section className="grid gap-2">
          <h4 className="text-sm font-semibold">{t("developer.parameters")}</h4>
          <div className="overflow-x-auto rounded-md border">
            <table className="w-full text-xs">
              <tbody>
                {op.parameters.map((parameter) => (
                  <tr key={`${parameter.in}:${parameter.name}`} className="border-t first:border-t-0">
                    <td className="px-3 py-1.5 font-mono whitespace-nowrap" dir="ltr">
                      {parameter.name}
                      {parameter.required && <span className="text-destructive"> *</span>}
                    </td>
                    <td className="px-3 py-1.5 text-muted-foreground">{parameter.in}</td>
                    <td className="px-3 py-1.5 text-muted-foreground" dir="auto">
                      {parameter.description}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {op.body && (
        <section className="grid gap-2">
          <h4 className="text-sm font-semibold">{t("developer.requestBody")}</h4>
          <Fields doc={doc} schema={op.body} />
        </section>
      )}

      <section className="grid gap-2">
        <h4 className="text-sm font-semibold">{t("developer.responses")}</h4>
        {op.responses.map((response) => (
          <div key={response.status} className="grid gap-2">
            <div className="flex items-center gap-2 text-sm">
              <Badge variant={response.status.startsWith("2") ? "success" : "outline"} className="font-mono">
                {response.status}
              </Badge>
              <span className="text-muted-foreground" dir="auto">
                {response.description}
              </span>
            </div>
            {response.schema && response.contentType === "application/json" && <Fields doc={doc} schema={response.schema} />}
          </div>
        ))}
      </section>

      <TryIt doc={doc} op={op} />
    </div>
  );
}

/** The API, read from its own OpenAPI document, with every call to try. */
export function DocsTab() {
  const { t } = useTranslation();
  const spec = useQuery({ queryKey: ["openapi"], queryFn: () => api<OpenApiDoc>("openapi.json"), staleTime: 300_000 });
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const all = useMemo(() => (spec.data ? operations(spec.data) : []), [spec.data]);
  const wanted = filter.trim().toLowerCase();
  const shown = all.filter(
    (op) => !wanted || `${op.method} ${op.path} ${op.summary} ${op.tag} ${op.id}`.toLowerCase().includes(wanted),
  );
  const tags = [...new Set(shown.map((op) => op.tag))];
  const current = all.find((op) => op.id === selected) ?? all[0];

  if (spec.isPending) return <Skeleton className="h-96" />;
  if (spec.isError || !spec.data) return <FormError error={spec.error} />;

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="font-semibold">
            {spec.data.info.title} <span className="font-mono text-xs text-muted-foreground">{spec.data.info.version}</span>
          </div>
          {spec.data.servers?.[0] && (
            <div className="truncate font-mono text-xs text-muted-foreground" dir="ltr">
              {spec.data.servers[0].url}
            </div>
          )}
        </div>
        <Button variant="outline" asChild>
          <a href={new URL("api/openapi.json", document.baseURI).href} download="z-ui-openapi.json">
            <DownloadIcon /> {t("developer.download")}
          </a>
        </Button>
      </div>
      {spec.data.info.description && (
        <Card>
          <CardContent className="p-4">
            <Prose text={spec.data.info.description} />
          </CardContent>
        </Card>
      )}
      <div className="grid gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
        <Card className="lg:sticky lg:top-20 lg:self-start">
          <CardContent className="grid gap-3 p-3">
            <div className="relative">
              <SearchIcon className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("developer.search")} className="ps-9" aria-label={t("developer.search")} />
            </div>
            <nav className="max-h-[60svh] overflow-y-auto" aria-label={t("developer.operations")}>
              {tags.length === 0 && <p className="p-2 text-sm text-muted-foreground">{t("developer.noOperations")}</p>}
              {tags.map((tag) => (
                <div key={tag} className="mb-3">
                  <div className="px-2 pb-1 text-xs font-medium text-muted-foreground">
                    {spec.data.tags?.find((item) => item.name === tag)?.description ?? tag}
                  </div>
                  <ul className="space-y-0.5">
                    {shown
                      .filter((op) => op.tag === tag)
                      .map((op) => (
                        <li key={op.id}>
                          <button
                            type="button"
                            onClick={() => setSelected(op.id)}
                            className={cn(
                              "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-start text-xs transition-colors hover:bg-accent",
                              current?.id === op.id && "bg-accent",
                            )}
                          >
                            <MethodBadge method={op.method} />
                            <span className="min-w-0 truncate" dir="auto" title={`${op.path} — ${op.summary}`}>
                              {op.summary}
                            </span>
                          </button>
                        </li>
                      ))}
                  </ul>
                </div>
              ))}
            </nav>
          </CardContent>
        </Card>
        <Card className="min-w-0">
          <CardContent className="p-5">{current && <Detail doc={spec.data} op={current} />}</CardContent>
        </Card>
      </div>
    </div>
  );
}
