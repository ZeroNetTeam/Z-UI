import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

/** The platform's checkbox in the theme's gold. */
export function Checkbox({ className, ...props }: Omit<ComponentProps<"input">, "type">) {
  return (
    <input
      type="checkbox"
      className={cn("size-4 shrink-0 cursor-pointer rounded accent-primary disabled:cursor-default disabled:opacity-50", className)}
      {...props}
    />
  );
}
