import { describe, expect, it } from "vitest";
import type { MigrateWarning } from "./api";
import { defaultOptions, groupWarnings, looksLikeDatabase, namesOf } from "./migrate";

const warning = (scope: string, name: string, code: string): MigrateWarning => ({ scope, name, code, detail: code });

describe("grouping warnings", () => {
  it("gathers the same trouble into one group, the commonest first", () => {
    const groups = groupWarnings([
      warning("inbound", "a", "renamed"),
      warning("user", "u1", "shared_sub_id"),
      warning("user", "u2", "shared_sub_id"),
      warning("user", "u3", "shared_sub_id"),
      warning("inbound", "b", "renamed"),
    ]);
    expect(groups.map((g) => [g.code, g.items.length])).toEqual([
      ["shared_sub_id", 3],
      ["renamed", 2],
    ]);
  });

  it("keeps a code apart per scope, and sorts equal counts by code", () => {
    const groups = groupWarnings([warning("user", "x", "renamed"), warning("inbound", "y", "renamed"), warning("user", "z", "a_code")]);
    expect(groups.map((g) => `${g.scope}/${g.code}`)).toEqual(["user/a_code", "inbound/renamed", "user/renamed"]);
  });

  it("names the first few things a group is about, once each", () => {
    const items = Array.from({ length: 12 }, (_, i) => warning("user", `u${i % 11}`, "no_inbound"));
    const names = namesOf({ code: "no_inbound", scope: "user", items });
    expect(names.shown).toHaveLength(8);
    expect(names.more).toBe(3);
    expect(namesOf({ code: "c", scope: "source", items: [warning("source", "", "c")] })).toEqual({ shown: [], more: 0 });
  });
});

describe("options and files", () => {
  it("brings everything but leaves the inbounds off", () => {
    expect(defaultOptions).toMatchObject({ inbounds: true, users: true, admins: true, usage: true, enable_inbounds: false, on_conflict: "skip" });
  });

  it("does not offer a text file as a database", () => {
    expect(looksLikeDatabase("x-ui.db")).toBe(true);
    expect(looksLikeDatabase("db.sqlite3")).toBe(true);
    expect(looksLikeDatabase("xray_config.json")).toBe(false);
    expect(looksLikeDatabase("backup.sql")).toBe(false);
  });
});
