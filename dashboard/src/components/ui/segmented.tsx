import { cn } from "@/lib/utils";

/** A few choices in one pill, one of them selected. */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string }[];
  label: string;
  className?: string;
}) {
  return (
    <div className={cn("flex w-fit max-w-full overflow-x-auto rounded-lg border p-1 text-sm", className)} role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          onClick={() => onChange(option.value)}
          className={cn(
            "shrink-0 rounded-md px-3 py-1 whitespace-nowrap transition-colors",
            option.value === value ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
