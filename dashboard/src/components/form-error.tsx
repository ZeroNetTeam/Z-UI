import { CircleAlertIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { errorText } from "@/lib/errors";

/** A failed request, in the reader's language. */
export function FormError({ error }: { error: unknown }) {
  const { t, i18n } = useTranslation();
  if (!error) return null;
  const text = errorText(error, t, i18n.language);
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      <CircleAlertIcon className="mt-0.5 size-4 shrink-0" />
      <span>{text}</span>
    </div>
  );
}
