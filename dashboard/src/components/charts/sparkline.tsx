import { useState } from "react";
import { cn } from "@/lib/utils";

export interface Series {
  /** One value per point; a gap is null. */
  values: (number | null)[];
  color: string;
}

/**
 * A line or two without axes, for a tile: 2px lines on a hairline floor,
 * a faint wash under the first. Move over it to read a value at that time.
 */
export function Sparkline({
  series,
  max,
  format,
  label,
  className,
}: {
  series: Series[];
  /** The top of the scale; the biggest value if absent. */
  max?: number;
  format: (value: number) => string;
  /** What it shows, for a screen reader. */
  label: string;
  className?: string;
}) {
  const [at, setAt] = useState<number | null>(null);
  const count = Math.max(0, ...series.map((s) => s.values.length));
  const top = max ?? Math.max(1e-9, ...series.flatMap((s) => s.values.filter((v): v is number => v !== null)));
  const x = (i: number) => (count <= 1 ? 50 : (i / (count - 1)) * 100);
  const y = (v: number) => 30 - Math.min(1, v / top) * 28;
  const path = (values: (number | null)[]) => {
    let d = "";
    let pen = false;
    values.forEach((v, i) => {
      if (v === null) {
        pen = false;
        return;
      }
      d += `${pen ? "L" : "M"}${x(i).toFixed(2)} ${y(v).toFixed(2)}`;
      pen = true;
    });
    return d;
  };
  if (count < 2) {
    return <div className={cn("h-10 rounded border-b border-border/70", className)} aria-label={label} role="img" />;
  }
  const readout = at === null ? null : series.map((s) => s.values[at]).filter((v): v is number => v !== null);
  return (
    <div
      className={cn("relative h-10", className)}
      role="img"
      aria-label={label}
      onPointerMove={(e) => {
        const box = e.currentTarget.getBoundingClientRect();
        setAt(Math.round(((e.clientX - box.left) / box.width) * (count - 1)));
      }}
      onPointerLeave={() => setAt(null)}
    >
      <svg viewBox="0 0 100 32" preserveAspectRatio="none" className="size-full overflow-visible" aria-hidden>
        <line x1="0" x2="100" y1="31" y2="31" stroke="var(--border)" strokeWidth="1" vectorEffect="non-scaling-stroke" />
        {series.map((s, index) => (
          <g key={s.color}>
            {index === 0 && (
              <path d={`${path(s.values)}L100 31L0 31Z`.replace(/^L/, "M")} fill={s.color} opacity="0.1" stroke="none" />
            )}
            <path d={path(s.values)} fill="none" stroke={s.color} strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
          </g>
        ))}
        {at !== null && <line x1={x(at)} x2={x(at)} y1="0" y2="31" stroke="var(--muted-foreground)" strokeWidth="1" vectorEffect="non-scaling-stroke" opacity="0.5" />}
      </svg>
      {at !== null &&
        series.map((s) => {
          const value = s.values[at];
          return value === null || value === undefined ? null : (
            <span
              key={s.color}
              className="pointer-events-none absolute size-2 -translate-x-1/2 -translate-y-1/2 rounded-full ring-2 ring-card"
              style={{ left: `${x(at)}%`, top: `${(y(value) / 32) * 100}%`, background: s.color }}
            />
          );
        })}
      {readout && readout.length > 0 && (
        <span className="tabular pointer-events-none absolute -top-5 end-0 rounded bg-popover px-1.5 text-[11px] text-foreground shadow" dir="ltr">
          {readout.map(format).join(" · ")}
        </span>
      )}
    </div>
  );
}
