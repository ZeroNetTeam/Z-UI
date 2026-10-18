import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { api, type AccessGroup, type Inbound } from "@/lib/api";

export function useGroups() {
  return useQuery({ queryKey: ["groups"], queryFn: () => api<AccessGroup[]>("groups") });
}

export function useInbounds() {
  return useQuery({ queryKey: ["inbounds"], queryFn: () => api<Inbound[]>("inbounds") });
}

/** Tick some of `items`. */
function Checklist({
  label,
  hint,
  items,
  value,
  onChange,
  empty,
}: {
  label: string;
  hint?: string;
  items: { id: number; label: string; detail?: string }[];
  value: number[];
  onChange: (value: number[]) => void;
  empty: string;
}) {
  return (
    <div className="grid gap-2">
      <Label>{label}</Label>
      {items.length === 0 ? (
        <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">{empty}</p>
      ) : (
        <div className="grid max-h-52 gap-1 overflow-y-auto rounded-md border p-1.5 sm:grid-cols-2">
          {items.map((item) => (
            <label key={item.id} className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 text-sm hover:bg-accent">
              <Checkbox
                checked={value.includes(item.id)}
                onChange={(e) => onChange(e.target.checked ? [...value, item.id] : value.filter((id) => id !== item.id))}
              />
              <span className="min-w-0 truncate">{item.label}</span>
              {item.detail && <span className="ms-auto shrink-0 text-xs text-muted-foreground" dir="auto">{item.detail}</span>}
            </label>
          ))}
        </div>
      )}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

export function GroupPicker({ value, onChange, hint }: { value: number[]; onChange: (value: number[]) => void; hint?: string }) {
  const { t } = useTranslation();
  const groups = useGroups();
  return (
    <Checklist
      label={t("users.groups")}
      hint={hint}
      empty={t("groups.empty")}
      value={value}
      onChange={onChange}
      items={(groups.data ?? []).map((group) => ({
        id: group.id,
        label: group.name,
        detail: group.all_inbounds ? t("groups.all") : t("groups.inboundCount", { count: group.inbound_ids.length }),
      }))}
    />
  );
}

export function InboundPicker({ value, onChange }: { value: number[]; onChange: (value: number[]) => void }) {
  const { t } = useTranslation();
  const inbounds = useInbounds();
  return (
    <Checklist
      label={t("groups.inbounds")}
      empty={t("inbounds.empty")}
      value={value}
      onChange={onChange}
      items={(inbounds.data ?? []).map((inbound) => ({
        id: inbound.id,
        label: inbound.remark,
        detail: `${inbound.protocol}:${inbound.port}`,
      }))}
    />
  );
}
