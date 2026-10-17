import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";

export function NotFoundPage() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center gap-4 py-24 text-center">
      <div className="tabular text-5xl font-semibold text-primary">404</div>
      <div className="text-muted-foreground">{t("notFound.title")}</div>
      <Button asChild variant="outline">
        <Link to="/">{t("notFound.back")}</Link>
      </Button>
    </div>
  );
}
