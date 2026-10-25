import { useMutation, useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useNavigate, useParams } from "react-router";
import { AuthShell } from "@/components/auth-shell";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { api, post, type Admin, type InviteView } from "@/lib/api";
import { useAuth, useAuthChanged } from "@/lib/auth";

/** Where an admin brought over from another panel sets a password, through the link they were given. */
export function InvitePage() {
  const { t } = useTranslation();
  const auth = useAuth();
  const authChanged = useAuthChanged();
  const navigate = useNavigate();
  const token = useParams().token ?? "";
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const who = useQuery({
    queryKey: ["invite", token],
    queryFn: () => api<InviteView>(`auth/invite/${encodeURIComponent(token)}`),
    retry: false,
  });
  const set = useMutation({ meta: { quiet: true },
    mutationFn: () => post<Admin>(`auth/invite/${encodeURIComponent(token)}`, { password }),
    onSuccess: async () => {
      await authChanged();
      navigate("/", { replace: true });
    },
  });
  // Someone signed in does not need a link.
  if (auth.data?.admin && !set.isSuccess) return <Navigate to="/" replace />;

  const mismatch = confirm.length > 0 && confirm !== password;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!mismatch) set.mutate();
  };
  return (
    <AuthShell title={t("auth.inviteTitle")} subtitle={who.data ? t("auth.inviteSubtitle", { username: who.data.username }) : ""}>
      {who.isError ? (
        <p className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm" dir="auto">
          {t("auth.inviteInvalid")}
        </p>
      ) : (
        <form onSubmit={submit} className="grid gap-4">
          <Field
            label={t("auth.password")}
            hint={t("auth.passwordHint")}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="new-password"
            minLength={8}
            dir="ltr"
            required
            autoFocus
          />
          <Field
            label={t("auth.confirmPassword")}
            type="password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            autoComplete="new-password"
            aria-invalid={mismatch}
            hint={mismatch ? t("auth.mismatch") : undefined}
            dir="ltr"
            required
          />
          <FormError error={set.error} />
          <Button type="submit" disabled={set.isPending || mismatch || !who.data}>
            {t("auth.inviteSet")}
          </Button>
        </form>
      )}
    </AuthShell>
  );
}
