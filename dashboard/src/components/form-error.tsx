import { CircleAlertIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ApiError } from "@/lib/api";

/** A failed request, in the reader's language. */
export function FormError({ error }: { error: unknown }) {
  const { t, i18n } = useTranslation();
  if (!error) return null;
  let text = t("errors.unknown");
  if (error instanceof ApiError) {
    const minutes = Math.ceil((error.retryAfter ?? 0) / 60);
    text = t(`errors.${error.code}`, {
      defaultValue: error.message,
      message: error.message,
      minutes: new Intl.NumberFormat(i18n.language).format(minutes),
    });
  }
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
