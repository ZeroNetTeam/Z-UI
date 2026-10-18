import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { FormError } from "@/components/form-error";

/** Ask before something that cannot be undone. */
export function Confirm({
  trigger,
  title,
  body,
  action,
  onConfirm,
}: {
  trigger: (open: () => void) => ReactNode;
  title: string;
  body: string;
  action: string;
  onConfirm: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  return (
    <>
      {trigger(() => {
        setError(null);
        setOpen(true);
      })}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title={title} className="max-w-sm">
          <p className="text-sm text-muted-foreground">{body}</p>
          <div className="mt-4 grid gap-3">
            <FormError error={error} />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setOpen(false)}>
                {t("common.cancel")}
              </Button>
              <Button
                variant="destructive"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    await onConfirm();
                    setOpen(false);
                  } catch (failure) {
                    setError(failure);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                {action}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
