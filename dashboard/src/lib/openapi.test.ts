import { describe, expect, it } from "vitest";
import { curl, example, operations, resolve, rows, typeName, url, type OpenApiDoc } from "./openapi";

const doc: OpenApiDoc = {
  info: { title: "T", version: "1" },
  tags: [{ name: "users" }, { name: "groups" }],
  paths: {
    "/groups": { get: { operationId: "groups_list", tags: ["groups"], summary: "List", responses: { "200": { description: "ok" } } } },
    "/users/{id}": {
      put: {
        operationId: "users_update",
        tags: ["users"],
        summary: "Change",
        "x-permission": "users.write",
        parameters: [{ name: "id", in: "path", required: true }],
        requestBody: { content: { "application/json": { schema: { $ref: "#/components/schemas/Input" } } } },
        responses: { "200": { description: "ok", content: { "application/json": { schema: { $ref: "#/components/schemas/View" } } } } },
      },
      get: { operationId: "users_get", tags: ["users"], summary: "Read", responses: {} },
    },
    "/auth/login": { post: { operationId: "auth_login", tags: ["auth"], security: [{}], responses: {} } },
  },
  components: {
    schemas: {
      Input: {
        type: "object",
        required: ["username"],
        properties: {
          username: { type: "string", description: "3 to 64" },
          limit: { type: ["integer", "null"], format: "int64" },
          status: { type: "string", enum: ["active", "disabled"] },
          plan: { oneOf: [{ type: "null" }, { $ref: "#/components/schemas/Plan" }] },
          tags: { type: "array", items: { type: "string" } },
        },
      },
      Plan: { type: "object", required: ["days"], properties: { days: { type: "integer" } } },
      View: { allOf: [{ $ref: "#/components/schemas/Input" }, { type: "object", required: ["id"], properties: { id: { type: "integer" } } }] },
      Loop: { type: "object", properties: { next: { $ref: "#/components/schemas/Loop" } } },
    },
  },
};

describe("openapi", () => {
  it("lists operations by the document's tag order, then path and method", () => {
    const list = operations(doc);
    expect(list.map((op) => `${op.method} ${op.path}`)).toEqual(["get /users/{id}", "put /users/{id}", "get /groups", "post /auth/login"]);
    const update = list.find((op) => op.id === "users_update")!;
    expect(update.permission).toBe("users.write");
    expect(update.body).toEqual({ $ref: "#/components/schemas/Input" });
    expect(update.responses[0]).toMatchObject({ status: "200", contentType: "application/json" });
    expect(list.find((op) => op.id === "auth_login")!.open).toBe(true);
    expect(list.find((op) => op.id === "groups_list")!.open).toBe(false);
    expect(list.find((op) => op.id === "groups_list")!.permission).toBeNull();
  });

  it("follows references", () => {
    expect(resolve(doc, { $ref: "#/components/schemas/Plan" }).required).toEqual(["days"]);
    expect(resolve(doc, undefined)).toEqual({});
    expect(resolve(doc, { $ref: "#/components/schemas/Missing" })).toEqual({});
  });

  it("names types in a line", () => {
    const input = resolve(doc, { $ref: "#/components/schemas/Input" }).properties!;
    expect(typeName(doc, input.username)).toBe("string");
    expect(typeName(doc, input.limit)).toBe("integer | null");
    expect(typeName(doc, input.status)).toBe('"active" | "disabled"');
    expect(typeName(doc, input.tags)).toBe("string[]");
    expect(typeName(doc, input.plan)).toBe("object | null");
    expect(typeName(doc, undefined)).toBe("any");
  });

  it("lays an object out as rows, merging allOf and indenting what nests", () => {
    const view = rows(doc, { $ref: "#/components/schemas/View" });
    expect(view.map((row) => row.name)).toEqual(["username", "limit", "status", "plan", "days", "tags", "id"]);
    expect(view.find((row) => row.name === "username")).toMatchObject({ required: true, description: "3 to 64", depth: 0 });
    expect(view.find((row) => row.name === "id")!.required).toBe(true);
    expect(view.find((row) => row.name === "days")!.depth).toBe(1);
    // A schema that refers to itself does not go on forever.
    expect(rows(doc, { $ref: "#/components/schemas/Loop" }).length).toBeLessThan(10);
  });

  it("makes a sample body", () => {
    expect(example(doc, { $ref: "#/components/schemas/Input" })).toEqual({
      username: "",
      limit: null,
      status: "active",
      plan: null,
      tags: [""],
    });
    expect(example(doc, { $ref: "#/components/schemas/View" })).toMatchObject({ id: 0, username: "" });
    expect(() => example(doc, { $ref: "#/components/schemas/Loop" })).not.toThrow();
  });

  it("fills a path and leaves the query to what was typed", () => {
    const parameters = [
      { name: "id", in: "path" as const },
      { name: "days", in: "query" as const },
      { name: "skip", in: "query" as const },
    ];
    expect(url({ path: "/users/{id}/usage" }, { id: "7", days: "30", skip: "" }, parameters)).toBe("/users/7/usage?days=30");
    expect(url({ path: "/users/{id}" }, { id: "a b" }, parameters)).toBe("/users/a%20b");
  });

  it("writes a curl command, quoting what could break out of it", () => {
    expect(curl("get", "https://p/api/users", null, true)).toBe(`curl -X GET 'https://p/api/users' \\\n  -H 'Authorization: Bearer '"$ZUI_KEY"`);
    const post = curl("post", "https://p/api/users", `{"note":"it's"}`, false);
    expect(post).toContain("-H 'Content-Type: application/json'");
    expect(post).toContain(`-d '{"note":"it'\\''s"}'`);
    expect(post).not.toContain("Authorization");
  });
});
