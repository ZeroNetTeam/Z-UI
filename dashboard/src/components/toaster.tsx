import { CircleAlertIcon, CircleCheckIcon, InfoIcon, XIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { dismiss, useToasts, type ToastKind } from "@/lib/toast";

const icons: Record<ToastKind, typeof InfoIcon> = {
  success: CircleCheckIcon,
  error: CircleAlertIcon,
  info: InfoIcon,
};

const tones: Record<ToastKind, string> = {
  success: "border-success/40 [&_svg.kind]:text-success",
  error: "border-destructive/50 [&_svg.kind]:text-destructive",
  info: "border-primary/40 [&_svg.kind]:text-primary",
};

/** The messages that tell what just happened: copied, saved, failed. */
export function Toaster() {
  const { t } = useTranslation();
  const toasts = useToasts();
  return (
    <div
      className="pointer-events-none fixed inset-x-0 bottom-4 z-[100] flex flex-col items-center gap-2 px-4"
      role="status"
      aria-live="polite"
    >
      {toasts.map((item) => {
        const Icon = icons[item.kind];
        return (
          <div
            key={item.id}
            role={item.kind === "error" ? "alert" : undefined}
            className={cn(
              "pointer-events-auto flex w-full max-w-md items-center gap-3 rounded-lg border bg-card px-4 py-3 text-sm text-card-foreground shadow-lg",
              tones[item.kind],
            )}
          >
            <Icon className="kind size-5 shrink-0" />
            <span className="min-w-0 flex-1 break-words" dir="auto">
              {item.text}
            </span>
            {item.action && (
              <button
                type="button"
                className="shrink-0 rounded-md bg-primary px-2.5 py-1 text-xs font-semibold text-primary-foreground hover:opacity-90"
                onClick={() => {
                  item.action?.run();
                  dismiss(item.id);
                }}
              >
                {item.action.label}
              </button>
            )}
            <button
              type="button"
              className="shrink-0 rounded-md p-1 text-muted-foreground hover:bg-accent"
              aria-label={t("common.close")}
              onClick={() => dismiss(item.id)}
            >
              <XIcon className="size-4" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
