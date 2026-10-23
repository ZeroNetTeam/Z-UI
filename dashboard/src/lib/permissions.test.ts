import { describe, expect, it } from "vitest";
import { can, everything, levelOf, minimal, withLevel } from "./permissions";

const reseller = {
  is_sudo: false,
  permissions: ["users.read", "users.write", "groups.read"],
};

describe("permissions", () => {
  it("asks the admin's own list, and sudo for sudo", () => {
    expect(can(reseller, "users.write")).toBe(true);
    expect(can(reseller, "groups.write")).toBe(false);
    expect(can(reseller, "core.read")).toBe(false);
    expect(can(reseller, "sudo")).toBe(false);
    expect(can({ ...reseller, is_sudo: true }, "sudo")).toBe(true);
    expect(can(null, "users.read")).toBe(false);
    expect(can(undefined, "users.read")).toBe(false);
  });

  it("reads the level of a section off a list", () => {
    expect(levelOf(reseller.permissions, "users")).toBe("write");
    expect(levelOf(reseller.permissions, "groups")).toBe("read");
    expect(levelOf(reseller.permissions, "core")).toBe("none");
    // `hosts` must not be confused with a longer name that starts alike.
    expect(levelOf(["hostsX.write"], "hosts")).toBe("none");
  });

  it("changes one section and leaves the others", () => {
    const up = withLevel(reseller.permissions, "groups", "write");
    expect(levelOf(up, "groups")).toBe("write");
    expect(levelOf(up, "users")).toBe("write");
    const down = withLevel(reseller.permissions, "users", "read");
    expect(levelOf(down, "users")).toBe("read");
    expect(down.includes("users.write")).toBe(false);
    const off = withLevel(reseller.permissions, "users", "none");
    expect(levelOf(off, "users")).toBe("none");
    expect(levelOf(off, "groups")).toBe("read");
  });

  it("sends the highest level of each section, in the server's order", () => {
    expect(minimal(["groups.read", "users.write", "users.read"])).toEqual(["users.write", "groups.read"]);
    expect(minimal([])).toEqual([]);
  });

  it("lists everything there is to give, the audit log only to read", () => {
    const all = everything();
    expect(all).toHaveLength(12);
    expect(all).toContain("audit.read");
    expect(all).not.toContain("audit.write");
    expect(all).toContain("users.write");
  });
});
