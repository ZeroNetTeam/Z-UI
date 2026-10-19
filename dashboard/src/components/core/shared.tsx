import { ArrowDownIcon, ArrowUpIcon, PencilIcon, Trash2Icon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Confirm } from "@/components/confirm";
import { Button } from "@/components/ui/button";
import type { CoreView } from "@/lib/api";
import type { Json } from "@/lib/core-form";

/** What every tab of the Core page is given. */
export interface TabProps {
  /** The settings being edited, unsaved changes included. */
  settings: Json;
  /** Change them: `change` is handed a copy to alter. */
  change: (change: (draft: Json) => void) => void;
  core: CoreView;
}

/** The list at `key` of `parent` (made if it is not there). */
export function listAt(parent: Json, key: string): unknown[] {
  if (!Array.isArray(parent[key])) parent[key] = [];
  return parent[key] as unknown[];
}

/** The object at `key` of `parent` (made if it is not there). */
export function objectAt(parent: Json, key: string): Json {
  const value = parent[key];
  if (!value || typeof value !== "object" || Array.isArray(value)) parent[key] = {};
  return parent[key] as Json;
}

/** Move the item at `index` one place up or down. */
export function move(list: unknown[], index: number, by: -1 | 1) {
  const target = index + by;
  if (target < 0 || target >= list.length) return;
  [list[index], list[target]] = [list[target], list[index]];
}

/** Move up, move down, edit and delete, for a row of an ordered list. */
export function RowActions({
  index,
  count,
  onMove,
  onEdit,
  onDelete,
  deleteTitle,
  deleteBody,
  extra,
}: {
  index: number;
  count: number;
  onMove?: (by: -1 | 1) => void;
  onEdit: () => void;
  onDelete: () => void;
  deleteTitle: string;
  deleteBody: string;
  extra?: React.ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex justify-end gap-0.5">
      {extra}
      {onMove && (
        <>
          <Button variant="ghost" size="icon" className="size-8" aria-label={t("core.moveUp")} disabled={index === 0} onClick={() => onMove(-1)}>
            <ArrowUpIcon />
          </Button>
          <Button variant="ghost" size="icon" className="size-8" aria-label={t("core.moveDown")} disabled={index === count - 1} onClick={() => onMove(1)}>
            <ArrowDownIcon />
          </Button>
        </>
      )}
      <Button variant="ghost" size="icon" className="size-8" aria-label={t("common.edit")} onClick={onEdit}>
        <PencilIcon />
      </Button>
      <Confirm
        title={deleteTitle}
        body={deleteBody}
        action={t("common.delete")}
        onConfirm={async () => onDelete()}
        trigger={(confirm) => (
          <Button variant="ghost" size="icon" className="size-8" aria-label={t("common.delete")} onClick={confirm}>
            <Trash2Icon className="text-destructive" />
          </Button>
        )}
      />
    </div>
  );
}

/** A checklist of choices in a bordered box, for a few tags. */
export function Checks({
  label,
  options,
  value,
  onChange,
  empty,
}: {
  label: string;
  options: { value: string; label: string }[];
  value: string[];
  onChange: (value: string[]) => void;
  empty?: string;
}) {
  return (
    <div className="grid gap-2">
      <span className="text-sm leading-none font-medium">{label}</span>
      {options.length === 0 ? (
        <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">{empty}</p>
      ) : (
        <div className="flex flex-wrap gap-x-4 gap-y-1.5 rounded-md border p-2.5">
          {options.map((option) => (
            <label key={option.value} className="flex cursor-pointer items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="size-4 accent-primary"
                checked={value.includes(option.value)}
                onChange={(e) => onChange(e.target.checked ? [...value, option.value] : value.filter((item) => item !== option.value))}
              />
              <span dir="auto">{option.label}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
