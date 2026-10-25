import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearToasts, dismiss, lifetime, toast, useToasts } from "./toast";
import { renderHook, act } from "@testing-library/react";

// The store without React: what `useToasts` shows is read through a hook.
function shown() {
  const { result } = renderHook(() => useToasts());
  return result;
}

describe("toast", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    clearToasts();
    vi.useRealTimers();
  });

  it("shows a message and takes it away by itself, an error later than a success", () => {
    const list = shown();
    act(() => void toast("Saved"));
    act(() => void toast("Failed", "error"));
    expect(list.current.map((t) => t.text)).toEqual(["Saved", "Failed"]);
    act(() => vi.advanceTimersByTime(lifetime("success", false) + 1));
    expect(list.current.map((t) => t.text)).toEqual(["Failed"]);
    act(() => vi.advanceTimersByTime(lifetime("error", false)));
    expect(list.current).toEqual([]);
    expect(lifetime("info", true)).toBeGreaterThan(lifetime("error", false));
  });

  it("does not stack the same words, keeps four at most, and dismisses on request", () => {
    const list = shown();
    act(() => {
      toast("Copied");
      toast("Copied");
    });
    expect(list.current).toHaveLength(1);
    act(() => {
      for (const n of [1, 2, 3, 4, 5]) toast(`m${n}`);
    });
    expect(list.current.map((t) => t.text)).toEqual(["m2", "m3", "m4", "m5"]);
    const id = list.current[0].id;
    act(() => dismiss(id));
    expect(list.current.map((t) => t.text)).toEqual(["m3", "m4", "m5"]);
  });
});
