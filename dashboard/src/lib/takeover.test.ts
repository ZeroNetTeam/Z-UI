import { describe, expect, it } from "vitest";
import { windowLeft } from "./takeover";

describe("the week the old panel is kept", () => {
  const now = 1_000_000;
  it("counts whole days and the hours after them", () => {
    expect(windowLeft(now + 6 * 86_400 + 5 * 3600 + 59, now)).toEqual({ days: 6, hours: 5, over: false });
    expect(windowLeft(now + 3599, now)).toEqual({ days: 0, hours: 0, over: false });
  });

  it("is over when the time has come, and never negative", () => {
    expect(windowLeft(now, now)).toEqual({ days: 0, hours: 0, over: true });
    expect(windowLeft(now - 500, now)).toEqual({ days: 0, hours: 0, over: true });
  });

  it("has nothing to count without an end", () => {
    expect(windowLeft(null, now)).toEqual({ days: 0, hours: 0, over: false });
  });
});
