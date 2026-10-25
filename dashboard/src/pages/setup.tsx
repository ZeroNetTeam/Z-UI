import { useMutation } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useNavigate, useSearchParams } from "react-router";
import { AuthShell } from "@/components/auth-shell";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { post, type Admin } from "@/lib/api";
import { useAuth, useAuthChanged } from "@/lib/auth";

export function SetupPage() {
  const { t } = useTranslation();
  const auth = useAuth();
  const authChanged = useAuthChanged();
  const navigate = useNavigate();
  const token = useSearchParams()[0].get("token") ?? "";
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");

  const setup = useMutation({ meta: { quiet: true },
    mutationFn: () => post<Admin>("setup", { token, username, password }),
    onSuccess: async () => {
      await authChanged();
      navigate("/", { replace: true });
    },
  });

  if (auth.data && !auth.data.setup_required) return <Navigate to="/" replace />;

  const mismatch = confirm.length > 0 && confirm !== password;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!mismatch) setup.mutate();
  };

  return (
    <AuthShell title={t("auth.setupTitle")} subtitle={t("auth.setupSubtitle")}>
      {!token ? (
        <p className="rounded-md border border-primary/30 bg-primary/5 p-3 text-sm">{t("auth.missingToken")}</p>
      ) : (
        <form onSubmit={submit} className="grid gap-4">
          <Field
            label={t("auth.username")}
            hint={t("auth.usernameHint")}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            dir="ltr"
            required
          />
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
          <FormError error={setup.error} />
          <Button type="submit" disabled={setup.isPending || mismatch}>
            {t("auth.create")}
          </Button>
        </form>
      )}
    </AuthShell>
  );
}
