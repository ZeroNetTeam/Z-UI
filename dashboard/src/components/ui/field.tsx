import { useId, type ComponentProps } from "react";
import { Input } from "./input";
import { Label } from "./label";

/** A labelled input with an optional hint underneath. */
export function Field({
  label,
  hint,
  ...props
}: ComponentProps<typeof Input> & { label: string; hint?: string }) {
  const id = useId();
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input id={id} {...props} />
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}
