import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { LockIcon } from "lucide-react";
import { pages } from "@/components/layout/navigation";
import { PageHeader } from "@/components/layout/layout";
import { Card, CardContent } from "@/components/ui/card";
import { can, useMe } from "@/lib/permissions";

/** Whether the admin may open the page at `to` (the navigation says what it needs). */
export function mayOpen(admin: Parameters<typeof can>[0], to: string): boolean {
  const item = pages.find((page) => page.to === to);
  if (!item) return true;
  if (item.sudo && !can(admin, "sudo")) return false;
  return !item.needs || can(admin, item.needs);
}

/** A page for those who may open it; a plain explanation for the rest. */
export function Gate({ to, children }: { to: string; children: ReactNode }) {
  const { t } = useTranslation();
  const admin = useMe();
  if (mayOpen(admin, to)) return children;
  return (
    <>
      <PageHeader title={t("forbidden.title")} />
      <Card>
        <CardContent className="flex flex-col items-center gap-3 p-10 text-center">
          <span className="flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <LockIcon className="size-5" />
          </span>
          <p className="max-w-sm text-sm text-muted-foreground">{t("forbidden.body")}</p>
        </CardContent>
      </Card>
    </>
  );
}
