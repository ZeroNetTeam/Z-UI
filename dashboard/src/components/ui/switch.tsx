import { cn } from "@/lib/utils";

export function Switch({
  checked,
  onCheckedChange,
  disabled,
  label,
  className,
}: {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-transparent transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50",
        checked ? "bg-primary" : "bg-input",
        className,
      )}
    >
      <span
        className={cn(
          "pointer-events-none block size-4 rounded-full bg-background shadow transition-transform",
          checked ? "ltr:translate-x-4 rtl:-translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );
}

/** A switch with its label beside it. */
export function SwitchField({
  label,
  hint,
  ...props
}: Omit<Parameters<typeof Switch>[0], "label"> & { label: string; hint?: string }) {
  return (
    <label className="flex items-start justify-between gap-4 rounded-md border p-3">
      <span className="grid gap-1">
        <span className="text-sm font-medium">{label}</span>
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </span>
      <Switch label={label} {...props} />
    </label>
  );
}
