import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { canonical } from "@/lib/core-form";
import type { Json } from "@/lib/core-form";
import type { CoreView } from "@/lib/api";

/** The whole settings as JSON, to read or edit by hand. */
export function JsonTab({ settings, replace, core }: { settings: Json; replace: (settings: Json) => void; core: CoreView }) {
  const { t } = useTranslation();
  const shown = JSON.stringify(settings, null, 2);
  const [text, setText] = useState(shown);
  const [problem, setProblem] = useState<string | null>(null);

  // What the forms change shows up here, unless this text is being edited.
  useEffect(() => {
    try {
      if (canonical(JSON.parse(text)) === canonical(settings)) return;
    } catch {
      // The text is not valid JSON: it is the reader's to finish.
      if (document.activeElement?.tagName === "TEXTAREA") return;
    }
    setText(shown);
    setProblem(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown]);

  const take = () => {
    try {
      const parsed: unknown = JSON.parse(text);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error(t("core.jsonNotObject"));
      setProblem(null);
      replace(parsed as Json);
    } catch (error) {
      setProblem((error as Error).message);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("core.jsonTitle")}</CardTitle>
        <CardDescription className="max-w-2xl">{t("core.jsonBody")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">
        <Textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          dir="ltr"
          spellCheck={false}
          aria-label={t("core.jsonTitle")}
          className="min-h-[28rem] font-mono text-xs leading-5"
        />
        {problem && (
          <p role="alert" className="text-sm text-destructive" dir="auto">
            {t("core.jsonInvalid", { message: problem })}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button onClick={take}>{t("core.jsonTake")}</Button>
          <Button
            variant="outline"
            onClick={() => {
              try {
                setText(JSON.stringify(JSON.parse(text), null, 2));
                setProblem(null);
              } catch (error) {
                setProblem((error as Error).message);
              }
            }}
          >
            {t("core.jsonFormat")}
          </Button>
          <Button variant="ghost" onClick={() => replace(structuredClone(core.defaults))}>
            {t("core.resetDefaults")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
