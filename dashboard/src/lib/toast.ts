import { useSyncExternalStore } from "react";

export type ToastKind = "success" | "error" | "info";

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
  /** A button on the toast, for what the person can still do (copy by hand, try again). */
  action?: { label: string; run: () => void };
}

let items: ToastItem[] = [];
let next = 1;
const listeners = new Set<() => void>();
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function emit() {
  for (const listener of listeners) listener();
}

export function dismiss(id: number) {
  const timer = timers.get(id);
  if (timer) clearTimeout(timer);
  timers.delete(id);
  items = items.filter((item) => item.id !== id);
  emit();
}

/** How long a toast stays: a failure or one with a button is given more time to be read. */
export function lifetime(kind: ToastKind, hasAction: boolean): number {
  if (hasAction) return 9000;
  return kind === "error" ? 7000 : 3500;
}

/** Show a message that goes away by itself. At most four stay; the oldest make room. */
export function toast(text: string, kind: ToastKind = "success", action?: ToastItem["action"]): number {
  const id = next++;
  items = [...items.filter((item) => item.text !== text).slice(-3), { id, kind, text, action }];
  emit();
  timers.set(
    id,
    setTimeout(() => dismiss(id), lifetime(kind, action !== undefined)),
  );
  return id;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useToasts(): ToastItem[] {
  return useSyncExternalStore(subscribe, () => items);
}

/** Every toast gone, for tests. */
export function clearToasts() {
  for (const id of [...timers.keys()]) dismiss(id);
  items = [];
  emit();
}
