import { cn } from "@/lib/utils";

/**
 * A figure against its limit. The fill carries how worried to be (calm,
 * warning, danger); the empty track is a paler step of the same colour, so
 * the state reads across the whole bar.
 */
export function Meter({ value, label, className }: { value: number | null; label: string; className?: string }) {
  if (value === null) return <div className={cn("h-2 rounded-full bg-muted", className)} aria-hidden />;
  const share = Math.min(1, Math.max(0, value));
  const tone = share < 0.7 ? "bg-success" : share < 0.9 ? "bg-warning" : "bg-destructive";
  const track = share < 0.7 ? "bg-success/20" : share < 0.9 ? "bg-warning/20" : "bg-destructive/20";
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(share * 100)}
      className={cn("h-2 overflow-hidden rounded-full", track, className)}
    >
      <div className={cn("h-full rounded-full transition-[width] duration-500", tone)} style={{ width: `${share * 100}%`, minWidth: share > 0 ? 6 : 0 }} />
    </div>
  );
}
