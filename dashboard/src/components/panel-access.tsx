import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2Icon, RotateCwIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { SelectField } from "@/components/ui/select";
import { api, post, put, type PanelInput, type PanelView } from "@/lib/api";
import { addressAfterRestart } from "@/lib/panel-address";
import { useCertificates } from "@/pages/inbounds";

const key = ["panel"] as const;

/** Wait for the panel to answer at `url`, then go there. A different origin
 * cannot be read, so any answer at all (even an opaque one) is enough. */
function goWhenUp(url: URL) {
  let tries = 0;
  const timer = setInterval(() => {
    tries += 1;
    const go = () => {
      clearInterval(timer);
      location.assign(url.toString());
    };
    if (tries > 25) return go();
    fetch(new URL("api/auth/state", url), { mode: "no-cors", cache: "no-store" }).then(go, () => undefined);
  }, 1000);
}

function PanelForm({ view }: { view: PanelView }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const certificates = useCertificates();
  const [listen, setListen] = useState(view.stored_listen);
  const [path, setPath] = useState(view.stored_base_path);
  const [certificate, setCertificate] = useState(view.stored_certificate_id === null ? "" : String(view.stored_certificate_id));
  const [restarting, setRestarting] = useState(false);
  const save = useMutation({
    mutationFn: () => {
      const payload: PanelInput = { listen, base_path: path, certificate_id: certificate === "" ? null : Number(certificate) };
      return put<PanelView>("panel", payload);
    },
    onSuccess: (saved) => client.setQueryData(key, saved),
  });
  const restart = useMutation({
    mutationFn: () => post("panel/restart"),
    onSuccess: () => {
      setRestarting(true);
      goWhenUp(addressAfterRestart(view, new URL(location.href)));
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };
  const dirty = listen !== view.stored_listen || path !== view.stored_base_path || certificate !== (view.stored_certificate_id === null ? "" : String(view.stored_certificate_id));
  const locked = (what: boolean) => (what ? t("panelAccess.locked") : undefined);
  return (
    <form onSubmit={submit} className="grid max-w-xl gap-4">
      <Field
        label={t("panelAccess.listen")}
        hint={locked(view.locked.listen) ?? t("panelAccess.listenHint")}
        value={listen}
        onChange={(e) => setListen(e.target.value)}
        disabled={view.locked.listen}
        dir="ltr"
        required
      />
      <Field
        label={t("panelAccess.path")}
        hint={locked(view.locked.base_path) ?? t("panelAccess.pathHint")}
        value={path}
        onChange={(e) => setPath(e.target.value)}
        disabled={view.locked.base_path}
        dir="ltr"
        required
      />
      <SelectField
        label={t("panelAccess.certificate")}
        hint={locked(view.locked.certificate) ?? t("panelAccess.certificateHint")}
        value={certificate}
        onChange={(e) => setCertificate(e.target.value)}
        disabled={view.locked.certificate}
        options={[
          { value: "", label: t("panelAccess.none") },
          ...(certificates.data ?? []).map((c) => ({ value: String(c.id), label: `${c.name} · ${c.domains.join(", ")}` })),
        ]}
      />
      <p className="text-xs text-muted-foreground">{t("panelAccess.warning")}</p>
      <FormError error={save.error ?? restart.error} />
      {save.isSuccess && !dirty && !view.restart_required && <p className="text-sm text-success">{t("panelAccess.saved")}</p>}
      {view.restart_required && !dirty && (
        <div role="status" className="grid gap-3 rounded-md border border-primary/40 bg-primary/5 p-3 text-sm">
          <p>{t("panelAccess.restartNeeded")}</p>
          {restarting ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2Icon className="size-4 animate-spin" /> {t("panelAccess.restarting")}
            </p>
          ) : (
            <Confirm
              title={t("panelAccess.restartTitle")}
              body={t("panelAccess.restartBody")}
              action={t("panelAccess.restart")}
              onConfirm={() => restart.mutateAsync()}
              trigger={(open) => (
                <Button type="button" className="justify-self-start" onClick={open}>
                  <RotateCwIcon /> {t("panelAccess.restart")}
                </Button>
              )}
            />
          )}
        </div>
      )}
      <Button type="submit" className="justify-self-start" disabled={save.isPending || !dirty}>
        {t("panelAccess.save")}
      </Button>
    </form>
  );
}

/** Where the panel listens, under which path and with which certificate. */
export function PanelAccessCard() {
  const { t } = useTranslation();
  const panel = useQuery({ queryKey: key, queryFn: () => api<PanelView>("panel") });
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("panelAccess.title")}</CardTitle>
        <CardDescription>{t("panelAccess.body")}</CardDescription>
      </CardHeader>
      <CardContent>
        {panel.data ? <PanelForm key={JSON.stringify(panel.data)} view={panel.data} /> : <FormError error={panel.error} />}
      </CardContent>
    </Card>
  );
}
