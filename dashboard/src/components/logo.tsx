import { useId } from "react";
import { cn } from "@/lib/utils";

/** The Z mark, in gold on graphite. */
export function Logo({ className }: { className?: string }) {
  // Its own gradient id: the desktop sidebar keeps a hidden copy, and a
  // shared id would point the visible one at a gradient that is not drawn.
  const gradient = `z-gold-${useId().replace(/:/g, "")}`;
  return (
    <svg viewBox="0 0 64 64" aria-hidden className={cn("size-8 shrink-0", className)}>
      <defs>
        <linearGradient id={gradient} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#f4d77a" />
          <stop offset=".55" stopColor="#d4a72c" />
          <stop offset="1" stopColor="#9c7414" />
        </linearGradient>
      </defs>
      <rect width="64" height="64" rx="14" fill="#111113" />
      <path d="M18 18h28l-20 24h20v4H18l20-24H18z" fill={`url(#${gradient})`} />
    </svg>
  );
}
