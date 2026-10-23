import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLinkIcon, Loader2Icon, RefreshCwIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { SwitchField } from "@/components/ui/switch";
import { api, post, put, type UpdateView } from "@/lib/api";
import { goWhenUp } from "@/lib/panel-address";
import { formatDate } from "@/lib/users";

const key = ["update"] as const;

function UpdateBody({ view }: { view: UpdateView }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [check, setCheck] = useState(view.check);
  const [proxy, setProxy] = useState(view.proxy);
  const [restarting, setRestarting] = useState(false);
  const looked = useMutation({
    mutationFn: () => post<UpdateView>("update/check"),
    onSuccess: (next) => client.setQueryData(key, next),
  });
  const apply = useMutation({
    mutationFn: () => post<{ version: string }>("update/apply"),
    onSuccess: () => {
      setRestarting(true);
      goWhenUp(new URL(location.href));
    },
  });
  const save = useMutation({
    mutationFn: () => put<UpdateView>("update", { check, proxy }),
    onSuccess: (next) => client.setQueryData(key, next),
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  const latest = view.latest;
  const dirty = check !== view.check || proxy !== view.proxy;
  return (
    <div className="grid max-w-xl gap-4">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="text-muted-foreground">{t("update.current")}</span>
        <Badge variant="secondary" dir="ltr">
          {view.current}
        </Badge>
        {latest?.available && (
          <>
            <span className="text-muted-foreground">{t("update.newest")}</span>
            <Badge dir="ltr">{latest.version}</Badge>
          </>
        )}
      </div>
      {latest?.available ? (
        <div className="grid gap-2 rounded-md border border-primary/40 bg-primary/5 p-3 text-sm">
          <p className="font-medium">{t("update.available", { version: latest.version })}</p>
          {latest.notes && (
            <p className="max-h-40 overflow-auto whitespace-pre-wrap text-xs text-muted-foreground" dir="auto">
              {latest.notes}
            </p>
          )}
          {latest.url && (
            <a href={latest.url} target="_blank" rel="noreferrer noopener" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
              <ExternalLinkIcon className="size-3.5" /> {t("update.notes")}
            </a>
          )}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          {view.checked_at > 0 ? t("update.upToDate", { when: formatDate(view.checked_at, i18n.language, true) }) : t("update.neverChecked")}
        </p>
      )}
      {view.error && <p className="text-sm text-destructive">{t("update.lastError", { error: view.error })}</p>}
      {view.container && <p className="text-sm text-muted-foreground">{t("update.container")}</p>}
      {!view.container && !view.can_apply && <p className="text-sm text-muted-foreground">{t("update.cannotApply")}</p>}
      <FormError error={looked.error ?? apply.error} />
      {restarting ? (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2Icon className="size-4 animate-spin" /> {t("update.restarting")}
        </p>
      ) : (
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" disabled={looked.isPending} onClick={() => looked.mutate()}>
            {looked.isPending ? <Loader2Icon className="animate-spin" /> : <RefreshCwIcon />} {t("update.checkNow")}
          </Button>
          {latest?.available && view.can_apply && (
            <Confirm
              title={t("update.applyTitle", { version: latest.version })}
              body={t("update.applyBody")}
              action={t("update.apply")}
              onConfirm={() => apply.mutateAsync()}
              trigger={(open) => (
                <Button type="button" onClick={open}>
                  {t("update.apply")}
                </Button>
              )}
            />
          )}
        </div>
      )}
      <form onSubmit={submit} className="grid gap-4 border-t pt-4">
        <SwitchField label={t("update.check")} hint={t("update.checkHint")} checked={check} onCheckedChange={setCheck} />
        <Field label={t("update.proxy")} hint={t("update.proxyHint")} value={proxy} onChange={(e) => setProxy(e.target.value)} dir="ltr" placeholder="socks5://127.0.0.1:10808" />
        <FormError error={save.error} />
        <Button type="submit" className="justify-self-start" disabled={save.isPending || !dirty}>
          {t("common.save")}
        </Button>
      </form>
    </div>
  );
}

/** Newer versions of the panel: what is out, and putting it in place. */
export function UpdateCard() {
  const { t } = useTranslation();
  const update = useQuery({ queryKey: key, queryFn: () => api<UpdateView>("update") });
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("update.title")}</CardTitle>
        <CardDescription>{t("update.body")}</CardDescription>
      </CardHeader>
      <CardContent>{update.data ? <UpdateBody key={JSON.stringify(update.data)} view={update.data} /> : <FormError error={update.error} />}</CardContent>
    </Card>
  );
}
