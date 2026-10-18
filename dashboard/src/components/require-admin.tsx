import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useLocation } from "react-router";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Logo } from "@/components/logo";
import { useAuth } from "@/lib/auth";

/** Let a signed-in admin through; send anyone else to setup or sign-in. */
export function RequireAdmin({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const auth = useAuth();
  const location = useLocation();

  if (auth.isPending) {
    return (
      <div className="grid min-h-svh place-items-center">
        <Logo className="size-10 animate-pulse" />
      </div>
    );
  }
  if (auth.isError) {
    return (
      <div className="grid min-h-svh place-items-center p-4">
        <div className="grid w-full max-w-sm gap-3">
          <FormError error={auth.error} />
          <Button variant="outline" onClick={() => void auth.refetch()}>
            {t("dashboard.retry")}
          </Button>
        </div>
      </div>
    );
  }
  if (auth.data.setup_required) return <Navigate to={`/setup${location.search}`} replace />;
  if (!auth.data.admin) return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  return children;
}
