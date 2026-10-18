import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";

/** Ask for one number (days to add, gigabytes to add), then act on it. */
export function AmountDialog({
  open,
  onOpenChange,
  title,
  label,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  label: string;
  onSubmit: (value: number) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    if (open) {
      setValue("");
      setError(null);
    }
  }, [open]);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    try {
      await onSubmit(Number(value));
      onOpenChange(false);
    } catch (failure) {
      setError(failure);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={title} className="max-w-sm">
        <form onSubmit={submit} className="grid gap-4">
          <Field label={label} type="number" min={0} step="any" value={value} onChange={(e) => setValue(e.target.value)} dir="ltr" required autoFocus />
          <FormError error={error} />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={busy || !(Number(value) > 0)}>
              {t("common.apply")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
