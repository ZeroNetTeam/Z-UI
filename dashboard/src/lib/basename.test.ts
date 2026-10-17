import { afterEach, describe, expect, it } from "vitest";
import { basename } from "./basename";

function withBase(href: string) {
  const base = document.createElement("base");
  base.href = href;
  document.head.prepend(base);
}

describe("basename", () => {
  afterEach(() => document.querySelectorAll("base").forEach((b) => b.remove()));

  it("is the path the server put in <base>", () => {
    withBase("http://panel.example:2053/s3cret/");
    expect(basename()).toBe("/s3cret");
  });

  it("is / at the root", () => {
    withBase("http://panel.example:2053/");
    expect(basename()).toBe("/");
  });
});
