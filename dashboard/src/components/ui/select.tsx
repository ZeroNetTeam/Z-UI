import { ChevronDownIcon } from "lucide-react";
import { useId, type ComponentProps } from "react";
import { cn } from "@/lib/utils";
import { Label } from "./label";

/** The platform's own select, styled: accessible and fine on phones. */
export function Select({ className, children, ...props }: ComponentProps<"select">) {
  return (
    <div className="relative">
      <select
        className={cn(
          "h-9 w-full appearance-none rounded-md border border-input bg-transparent ps-3 pe-8 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/40 disabled:opacity-50 [&>option]:bg-popover",
          className,
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDownIcon className="pointer-events-none absolute end-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
    </div>
  );
}

export function SelectField({
  label,
  hint,
  options,
  ...props
}: ComponentProps<"select"> & {
  label: string;
  hint?: string;
  options: { value: string; label: string }[];
}) {
  const id = useId();
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Select id={id} {...props}>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </Select>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}
