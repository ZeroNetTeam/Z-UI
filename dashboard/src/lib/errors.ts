import type { TFunction } from "i18next";
import { ApiError } from "@/lib/api";

/** A failed request, in the reader's language. */
export function errorText(error: unknown, t: TFunction, language: string): string {
  if (error instanceof ApiError) {
    const minutes = Math.ceil((error.retryAfter ?? 0) / 60);
    return t(`errors.${error.code}`, {
      defaultValue: error.message,
      message: error.message,
      minutes: new Intl.NumberFormat(language).format(minutes),
    });
  }
  return t("errors.unknown");
}
