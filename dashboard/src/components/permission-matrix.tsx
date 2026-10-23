import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { everything, levelOf, minimal, readOnlySections, sections, withLevel, type Level, type Section } from "@/lib/permissions";
import { cn } from "@/lib/utils";

const rank: Record<Level, number> = { none: 0, read: 1, write: 2 };

/**
 * Pick, for each part of the panel, none, view or manage. `limit` is what may
 * be given at most (a key cannot have more than its admin); absent, anything.
 */
export function PermissionMatrix({
  value,
  onChange,
  limit,
}: {
  value: string[];
  onChange: (value: string[]) => void;
  limit?: string[];
}) {
  const { t } = useTranslation();
  const most = (section: Section): Level =>
    limit ? levelOf(limit, section) : readOnlySections.includes(section) ? "read" : "write";
  const allowed = (cap: string[]) => cap.filter((name) => rank[levelOf(limit ?? everything(), name.split(".")[0] as Section)] >= rank[name.endsWith(".write") ? "write" : "read"]);
  const reads = sections.filter((section) => rank[most(section)] >= 1).map((section) => `${section}.read`);
  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap gap-1">
        <Button type="button" variant="outline" size="sm" onClick={() => onChange([])}>
          {t("access.presetNone")}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={() => onChange(reads)}>
          {t("access.presetRead")}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={() => onChange(minimal(allowed(everything())))}>
          {t("access.presetAll")}
        </Button>
      </div>
      <div className="divide-y rounded-lg border" role="group" aria-label={t("access.permissions")}>
        {sections.map((section) => {
          const level = levelOf(value, section);
          const top = most(section);
          const options = (["none", "read", ...(readOnlySections.includes(section) ? [] : ["write"])] as Level[]);
          return (
            <div key={section} className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-3 py-2">
              <div className="min-w-0 flex-1 basis-48">
                <div className="text-sm font-medium">{t(`access.section.${section}`)}</div>
                <div className="text-xs text-muted-foreground">{t(`access.sectionHint.${section}`)}</div>
              </div>
              <div className="flex rounded-lg border p-0.5 text-xs" role="radiogroup" aria-label={t(`access.section.${section}`)}>
                {options.map((option) => (
                  <button
                    key={option}
                    type="button"
                    role="radio"
                    aria-checked={option === level}
                    disabled={rank[option] > rank[top]}
                    onClick={() => onChange(withLevel(value, section, option))}
                    className={cn(
                      "rounded-md px-2.5 py-1 whitespace-nowrap transition-colors disabled:cursor-not-allowed disabled:opacity-40",
                      option === level ? "bg-primary text-primary-foreground" : "text-muted-foreground enabled:hover:bg-accent",
                    )}
                  >
                    {t(`access.level.${option}`)}
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
