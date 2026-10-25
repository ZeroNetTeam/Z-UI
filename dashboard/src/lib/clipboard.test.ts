import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/i18n", () => ({ default: { t: (key: string, options?: { what?: string }) => (options?.what ? `${key}:${options.what}` : key) } }));

import { copyAndTell, copyText } from "./clipboard";
import { clearToasts } from "./toast";
import { renderHook } from "@testing-library/react";
import { useToasts } from "./toast";

function setSecure(value: boolean) {
  Object.defineProperty(window, "isSecureContext", { value, configurable: true });
}

describe("copying", () => {
  const original = { clipboard: Object.getOwnPropertyDescriptor(navigator, "clipboard"), exec: document.execCommand };
  beforeEach(() => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
  });
  afterEach(() => {
    clearToasts();
    if (original.clipboard) Object.defineProperty(navigator, "clipboard", original.clipboard);
    else Reflect.deleteProperty(navigator, "clipboard");
    document.execCommand = original.exec;
  });

  it("uses the clipboard where the page may", async () => {
    setSecure(true);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    expect(await copyText("hello")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello");
  });

  it("copies by hand on a page that has no clipboard (plain HTTP), and leaves nothing behind", async () => {
    setSecure(false);
    let selected = "";
    document.execCommand = vi.fn(() => {
      selected = (document.activeElement as HTMLTextAreaElement | null)?.value ?? (document.querySelector("textarea") as HTMLTextAreaElement).value;
      return true;
    });
    expect(await copyText("https://panel.example/sub/abc")).toBe(true);
    expect(document.execCommand).toHaveBeenCalledWith("copy");
    expect(selected).toBe("https://panel.example/sub/abc");
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("falls back when the clipboard refuses, and says so when nothing works", async () => {
    setSecure(true);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn().mockRejectedValue(new Error("no focus")) }, configurable: true });
    document.execCommand = vi.fn(() => false);
    expect(await copyText("x")).toBe(false);
    expect(document.execCommand).toHaveBeenCalledOnce();
  });

  it("tells the person how it went, with a button to try again when it failed", async () => {
    setSecure(false);
    const list = renderHook(() => useToasts()).result;
    document.execCommand = vi.fn(() => true);
    await copyAndTell("a", "Link");
    expect(list.current.map((t) => [t.kind, t.text])).toEqual([["success", "toast.copiedWhat:Link"]]);
    clearToasts();
    document.execCommand = vi.fn(() => false);
    await copyAndTell("a");
    expect(list.current).toHaveLength(1);
    expect(list.current[0]).toMatchObject({ kind: "error", text: "toast.copyFailed" });
    expect(list.current[0].action?.label).toBe("toast.tryAgain");
  });
});
