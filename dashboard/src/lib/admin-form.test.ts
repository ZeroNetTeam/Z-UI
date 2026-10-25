import { describe, expect, it } from "vitest";
import { emptyAdmin, fromAdmin, quotaPercent, toAdminPayload } from "./admin-form";
import type { AdminRow } from "./api";

const GB = 1024 ** 3;

const row: AdminRow = {
  id: 2,
  username: "shop",
  is_sudo: false,
  enabled: true,
  role_id: 5,
  role: "Reseller",
  totp_enabled: false,
  pending: false,
  last_login_at: null,
  created_at: 0,
  user_limit: 30,
  data_quota: 500 * GB,
  group_ids: [1, 3],
  note: "ali's shop",
  users: 4,
  allocated: 100 * GB,
  unlimited: 0,
  used: 0,
  keys: 0,
};

describe("admin form", () => {
  it("round-trips what an admin holds", () => {
    const form = fromAdmin(row);
    expect(form).toMatchObject({ roleId: "5", userLimit: "30", dataQuotaGb: "500", groupIds: [1, 3], password: "" });
    const payload = toAdminPayload(form, false);
    expect(payload).toEqual({
      is_sudo: false,
      role_id: 5,
      enabled: true,
      user_limit: 30,
      data_quota: 500 * GB,
      group_ids: [1, 3],
      note: "ali's shop",
    });
    expect("password" in payload).toBe(false);
    expect("username" in payload).toBe(false);
  });

  it("sends a new admin's name and password, and an old one's only to change it", () => {
    const made = toAdminPayload({ ...emptyAdmin(5), username: " newone ", password: "a-long-password" }, true);
    expect(made.username).toBe("newone");
    expect(made.password).toBe("a-long-password");
    const changed = toAdminPayload({ ...fromAdmin(row), password: "another-long-one" }, false);
    expect(changed.password).toBe("another-long-one");
  });

  it("gives a sudo admin no role, quota or group limit", () => {
    const sudo = toAdminPayload({ ...fromAdmin(row), isSudo: true }, false);
    expect(sudo).toMatchObject({ is_sudo: true, role_id: null, user_limit: 0, data_quota: 0, group_ids: [] });
  });

  it("reads the numbers the way they are typed", () => {
    const form = { ...fromAdmin(row), userLimit: " 7.6 ", dataQuotaGb: "1.5", roleId: "" };
    const payload = toAdminPayload(form, false);
    expect(payload.user_limit).toBe(8);
    expect(payload.data_quota).toBe(1.5 * GB);
    expect(payload.role_id).toBeNull();
    expect(toAdminPayload({ ...form, userLimit: "-3" }, false).user_limit).toBe(0);
    expect(toAdminPayload({ ...form, userLimit: "abc" }, false).user_limit).toBe(0);
  });

  it("says how much of a quota is spent", () => {
    expect(quotaPercent(100 * GB, 500 * GB)).toBe(20);
    expect(quotaPercent(600 * GB, 500 * GB)).toBe(100);
    expect(quotaPercent(5, 0)).toBeNull();
  });
});
