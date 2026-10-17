import { HammerIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/layout/layout";
import { Card, CardContent } from "@/components/ui/card";
import type { NavItem } from "@/components/layout/navigation";

/** A page the roadmap has not reached yet. */
export function PlaceholderPage({ item }: { item: NavItem }) {
  const { t, i18n } = useTranslation();
  const page = t(`nav.${item.label}`);
  return (
    <>
      <PageHeader title={page} />
      <Card>
        <CardContent className="flex flex-col items-center gap-3 p-10 text-center">
          <span className="flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <HammerIcon className="size-5" />
          </span>
          <div className="font-medium">{t("placeholder.title")}</div>
          <p className="max-w-sm text-sm text-muted-foreground">
            {t("placeholder.body", {
              page,
              phase: new Intl.NumberFormat(i18n.language).format(item.phase ?? 0),
            })}
          </p>
        </CardContent>
      </Card>
    </>
  );
}
