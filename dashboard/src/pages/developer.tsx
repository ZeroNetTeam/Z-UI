import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router";
import { DocsTab } from "@/components/api-docs";
import { KeysTab } from "@/components/api-keys";
import { PageHeader } from "@/components/layout/layout";
import { Segmented } from "@/components/ui/segmented";
import { useCan } from "@/lib/permissions";
import { Card, CardContent } from "@/components/ui/card";
import { LockIcon } from "lucide-react";

type Tab = "docs" | "keys";

/** The API: how to reach the panel from a script, and the keys to do it with. */
export function DeveloperPage() {
  const { t } = useTranslation();
  const mayHaveKeys = useCan("api.write");
  const [params, setParams] = useSearchParams();
  const tab: Tab = params.get("tab") === "keys" ? "keys" : "docs";
  const setTab = (next: Tab) => setParams(next === "keys" ? { tab: "keys" } : {}, { replace: true });

  return (
    <>
      <PageHeader title={t("developer.title")} subtitle={t("developer.subtitle")} />
      <Segmented
        label={t("developer.title")}
        value={tab}
        onChange={setTab}
        options={[
          { value: "docs", label: t("developer.tabDocs") },
          { value: "keys", label: t("developer.tabKeys") },
        ]}
        className="mb-4"
      />
      {tab === "docs" ? (
        <DocsTab />
      ) : mayHaveKeys ? (
        <KeysTab />
      ) : (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 p-10 text-center">
            <LockIcon className="size-8 text-primary/60" />
            <p className="max-w-sm text-sm text-muted-foreground">{t("developer.noKeyAccess")}</p>
          </CardContent>
        </Card>
      )}
    </>
  );
}
