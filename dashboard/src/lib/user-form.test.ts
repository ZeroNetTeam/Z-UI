import { describe, expect, it } from "vitest";
import type { User, UserTemplate } from "./api";
import { emptyForm, expiryIn, fromUser, toPayload, withTemplate } from "./user-form";
import { GB } from "./users";

const user: User = {
  id: 1,
  username: "alice",
  status: "limited",
  data_limit: 50 * GB,
  used_traffic: 50 * GB,
  lifetime_traffic: 80 * GB,
  expire_at: 2_000_000_000,
  on_hold_duration: null,
  on_hold_timeout: null,
  reset_strategy: "month",
  last_reset_at: null,
  ip_limit: 2,
  speed_up: 1_250_000,
  speed_down: 2_500_000,
  note: "office",
  online_at: null,
  online: false,
  group_ids: [1],
  next_plan: { data_limit: 100 * GB, expire_days: 30, carry_over: true },
  hwid_limit: 3,
  sub_updated_at: null,
  sub_user_agent: "",
  uuid: "u",
  password: "p",
  sub_token: "t",
  created_at: 0,
  updated_at: 0,
};

describe("the user form", () => {
  it("round-trips a user without moving its expiry", () => {
    const payload = toPayload(fromUser(user));
    expect(payload).toMatchObject({
      username: "alice",
      status: "active",
      data_limit: 50 * GB,
      expire_at: 2_000_000_000,
      reset_strategy: "month",
      ip_limit: 2,
      hwid_limit: 3,
      speed_up: 1_250_000,
      speed_down: 2_500_000,
      group_ids: [1],
      next_plan: { data_limit: 100 * GB, expire_days: 30, carry_over: true },
    });
    expect(toPayload({ ...fromUser(user), nextPlan: false }).next_plan).toBeNull();
  });

  it("holds an on-hold user's days until it connects", () => {
    const form = { ...emptyForm([1]), username: "bob", status: "on_hold" as const, onHoldDays: "30", onHoldTimeoutDays: "7", expireAt: 5 };
    const payload = toPayload(form, 1000);
    expect(payload.expire_at).toBeNull();
    expect(payload.on_hold_duration).toBe(30 * 86_400);
    expect(payload.on_hold_timeout).toBe(1000 + 7 * 86_400);
  });

  it("reads blanks as no limit", () => {
    const payload = toPayload({ ...emptyForm(), username: " carol ", dataLimitGb: "", ipLimit: "-3" });
    expect(payload).toMatchObject({ username: "carol", data_limit: 0, ip_limit: 0, expire_at: null, on_hold_duration: null });
    expect(expiryIn("", 10)).toBeNull();
    expect(expiryIn("2", 10)).toBe(10 + 2 * 86_400);
  });

  it("takes a template's limits", () => {
    const template: UserTemplate = {
      id: 1,
      name: "monthly",
      data_limit: 100 * GB,
      expire_days: 30,
      on_hold: true,
      reset_strategy: "no_reset",
      ip_limit: 1,
      speed_up: 0,
      speed_down: 0,
      hwid_limit: 2,
      group_ids: [],
    };
    const form = withTemplate({ ...emptyForm([4]), username: "dan" }, template, 0);
    expect(form).toMatchObject({ username: "dan", status: "on_hold", dataLimitGb: "100", onHoldDays: "30", expireAt: null, groupIds: [4] });
    const dated = withTemplate(form, { ...template, on_hold: false }, 0);
    expect(dated).toMatchObject({ status: "active", expireAt: 30 * 86_400 });
  });
});
