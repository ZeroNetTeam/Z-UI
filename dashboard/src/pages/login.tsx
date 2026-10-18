import { useMutation } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useLocation, useNavigate } from "react-router";
import { AuthShell } from "@/components/auth-shell";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { ApiError, post, type Admin } from "@/lib/api";
import { useAuth, useAuthChanged } from "@/lib/auth";

export function LoginPage() {
  const { t } = useTranslation();
  const auth = useAuth();
  const authChanged = useAuthChanged();
  const navigate = useNavigate();
  const from = (useLocation().state as { from?: string } | null)?.from ?? "/";
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [needCode, setNeedCode] = useState(false);

  const login = useMutation({
    mutationFn: () => post<Admin>("auth/login", { username, password, code: needCode ? code : undefined }),
    onSuccess: async () => {
      await authChanged();
      navigate(from, { replace: true });
    },
    onError: (error) => {
      if (error instanceof ApiError && error.code === "totp_required") setNeedCode(true);
    },
  });

  if (auth.data?.setup_required) return <Navigate to="/setup" replace />;
  if (auth.data?.admin) return <Navigate to={from} replace />;

  const submit = (event: FormEvent) => {
    event.preventDefault();
    login.mutate();
  };
  const askingForCode = login.error instanceof ApiError && login.error.code === "totp_required";

  return (
    <AuthShell title={t("auth.signInTitle")} subtitle={t("auth.signInSubtitle")}>
      <form onSubmit={submit} className="grid gap-4">
        <Field
          label={t("auth.username")}
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoComplete="username"
          autoCapitalize="none"
          spellCheck={false}
          dir="ltr"
          required
          autoFocus
        />
        <Field
          label={t("auth.password")}
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoComplete="current-password"
          dir="ltr"
          required
        />
        {needCode && (
          <Field
            label={t("auth.code")}
            hint={t("auth.codeHint")}
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
            inputMode="numeric"
            autoComplete="one-time-code"
            dir="ltr"
            className="text-center tracking-[0.4em]"
            required
            autoFocus
          />
        )}
        {!askingForCode && <FormError error={login.error} />}
        <Button type="submit" disabled={login.isPending}>
          {login.isPending ? t("auth.signingIn") : t("auth.signIn")}
        </Button>
      </form>
    </AuthShell>
  );
}
