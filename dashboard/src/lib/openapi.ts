/** Reading the panel's own OpenAPI document, for the API page. */

export interface Schema {
  $ref?: string;
  type?: string | string[];
  format?: string;
  description?: string;
  properties?: Record<string, Schema>;
  required?: string[];
  items?: Schema;
  enum?: unknown[];
  oneOf?: Schema[];
  anyOf?: Schema[];
  allOf?: Schema[];
  additionalProperties?: Schema | boolean;
  default?: unknown;
  example?: unknown;
}

export interface Parameter {
  name: string;
  in: "path" | "query" | "header" | "cookie";
  required?: boolean;
  description?: string;
  schema?: Schema;
}

interface RawOperation {
  operationId: string;
  tags?: string[];
  summary?: string;
  description?: string;
  parameters?: Parameter[];
  requestBody?: { content?: Record<string, { schema?: Schema }> };
  responses?: Record<string, { description?: string; content?: Record<string, { schema?: Schema }> }>;
  security?: Record<string, unknown>[];
  "x-permission"?: string;
}

export interface OpenApiDoc {
  info: { title: string; description?: string; version: string };
  servers?: { url: string }[];
  tags?: { name: string; description?: string }[];
  paths: Record<string, Partial<Record<string, RawOperation>>>;
  components?: { schemas?: Record<string, Schema> };
}

export interface Operation {
  id: string;
  method: string;
  path: string;
  tag: string;
  summary: string;
  description: string;
  /** What it needs: `users.read`, `sudo`, `session`… none when any sign-in does. */
  permission: string | null;
  /** Open to anyone, with no sign-in. */
  open: boolean;
  parameters: Parameter[];
  body: Schema | null;
  responses: { status: string; description: string; schema: Schema | null; contentType: string | null }[];
}

const methods = ["get", "post", "put", "patch", "delete"] as const;

/** Every operation, grouped as the document's tags are ordered, then by path. */
export function operations(doc: OpenApiDoc): Operation[] {
  const order = (doc.tags ?? []).map((tag) => tag.name);
  const all: Operation[] = [];
  for (const [path, item] of Object.entries(doc.paths)) {
    for (const method of methods) {
      const raw = item[method];
      if (!raw) continue;
      const json = raw.requestBody?.content?.["application/json"]?.schema;
      all.push({
        id: raw.operationId,
        method,
        path,
        tag: raw.tags?.[0] ?? "other",
        summary: raw.summary ?? "",
        description: raw.description ?? "",
        permission: raw["x-permission"] ?? null,
        open: JSON.stringify(raw.security) === "[{}]",
        parameters: raw.parameters ?? [],
        body: json ?? null,
        responses: Object.entries(raw.responses ?? {}).map(([status, response]) => {
          const [contentType, media] = Object.entries(response.content ?? {})[0] ?? [null, undefined];
          return { status, description: response.description ?? "", schema: media?.schema ?? null, contentType };
        }),
      });
    }
  }
  const rank = (tag: string) => {
    const at = order.indexOf(tag);
    return at < 0 ? order.length : at;
  };
  return all.sort((a, b) => rank(a.tag) - rank(b.tag) || a.path.localeCompare(b.path) || methods.indexOf(a.method as never) - methods.indexOf(b.method as never));
}

/** Follow a `$ref` to the schema it names; anything else comes back as it is. */
export function resolve(doc: OpenApiDoc, schema: Schema | undefined): Schema {
  let current = schema ?? {};
  for (let hops = 0; current.$ref && hops < 8; hops++) {
    const name = current.$ref.replace("#/components/schemas/", "");
    current = doc.components?.schemas?.[name] ?? {};
  }
  return current;
}

/** Whether a value may be `null`: a type list with it, or a variant that is only it. */
function nullable(doc: OpenApiDoc, schema: Schema | undefined): boolean {
  const s = resolve(doc, schema);
  if (Array.isArray(s.type) && s.type.includes("null")) return true;
  return (s.oneOf ?? s.anyOf ?? []).some((part) => resolve(doc, part).type === "null");
}

/** The schema with its `allOf` parts folded together, and `X or null` read as `X`. */
function merged(doc: OpenApiDoc, schema: Schema | undefined): Schema {
  const base = resolve(doc, schema);
  const variants = base.oneOf ?? base.anyOf;
  if (variants) {
    const real = variants.filter((part) => resolve(doc, part).type !== "null");
    if (real.length === 1 && variants.length > real.length) return merged(doc, real[0]);
  }
  if (!base.allOf) return base;
  const out: Schema = { ...base, properties: { ...base.properties }, required: [...(base.required ?? [])] };
  delete out.allOf;
  for (const part of base.allOf) {
    const piece = merged(doc, part);
    out.properties = { ...out.properties, ...piece.properties };
    out.required = [...(out.required ?? []), ...(piece.required ?? [])];
    out.type = out.type ?? piece.type;
    out.description = out.description ?? piece.description;
    if (piece.oneOf) out.oneOf = piece.oneOf;
  }
  return out;
}

/** How a schema reads in one line: `integer`, `string[]`, `object`… */
export function typeName(doc: OpenApiDoc, schema: Schema | undefined): string {
  const s = merged(doc, schema);
  const kinds = (Array.isArray(s.type) ? s.type : s.type ? [s.type] : []).filter((kind) => kind !== "null");
  const orNull = nullable(doc, schema);
  let name: string;
  if (s.enum) name = s.enum.map((value) => JSON.stringify(value)).join(" | ");
  else if (s.oneOf || s.anyOf) {
    const parts = (s.oneOf ?? s.anyOf ?? []).map((part) => typeName(doc, part)).filter((part) => part !== "null");
    name = parts.join(" | ");
  } else if (kinds[0] === "array") name = `${typeName(doc, s.items)}[]`;
  else if (kinds.length) name = kinds.join(" | ");
  else if (s.properties) name = "object";
  else name = "any";
  return orNull && !name.includes("null") ? `${name} | null` : name;
}

export interface Row {
  name: string;
  type: string;
  required: boolean;
  description: string;
  depth: number;
}

/** The fields of an object schema, nested ones indented, as a table. */
export function rows(doc: OpenApiDoc, schema: Schema | undefined, depth = 0, seen: string[] = []): Row[] {
  const ref = schema?.$ref;
  if (ref && seen.includes(ref)) return [];
  const s = merged(doc, schema);
  if (s.type === "array" || (Array.isArray(s.type) && s.type.includes("array"))) {
    return rows(doc, s.items, depth, ref ? [...seen, ref] : seen);
  }
  const out: Row[] = [];
  for (const [name, field] of Object.entries(s.properties ?? {})) {
    const resolved = merged(doc, field);
    out.push({
      name,
      type: typeName(doc, field),
      required: (s.required ?? []).includes(name),
      description: field.description ?? resolved.description ?? "",
      depth,
    });
    const inner = resolved.type === "array" ? resolved.items : field;
    if (inner && depth < 3 && Object.keys(merged(doc, inner).properties ?? {}).length > 0) {
      out.push(...rows(doc, inner, depth + 1, ref ? [...seen, ref] : seen));
    }
  }
  return out;
}

/** A sample value for a schema: what a request body might start from. */
export function example(doc: OpenApiDoc, schema: Schema | undefined, depth = 0): unknown {
  // A field that may be left out is, in a sample, left out.
  if (depth > 0 && nullable(doc, schema)) return null;
  const s = merged(doc, schema);
  if (s.example !== undefined) return s.example;
  if (s.default !== undefined) return s.default;
  if (s.enum) return s.enum[0];
  if (s.oneOf || s.anyOf) {
    const first = (s.oneOf ?? s.anyOf ?? []).find((part) => resolve(doc, part).type !== "null");
    return example(doc, first, depth + 1);
  }
  const kind = Array.isArray(s.type) ? s.type.find((k) => k !== "null") : s.type;
  if (kind === "array") return depth > 3 ? [] : [example(doc, s.items, depth + 1)];
  if (kind === "integer" || kind === "number") return 0;
  if (kind === "boolean") return false;
  if (kind === "string") return "";
  if (s.properties || kind === "object") {
    if (depth > 3) return {};
    return Object.fromEntries(Object.entries(s.properties ?? {}).map(([name, field]) => [name, example(doc, field, depth + 1)]));
  }
  return null;
}

/** `/users/{id}` with its parameters filled in, and the query after it. */
export function url(op: Pick<Operation, "path">, values: Record<string, string>, parameters: Parameter[]): string {
  let path = op.path;
  for (const parameter of parameters.filter((p) => p.in === "path")) {
    path = path.replace(`{${parameter.name}}`, encodeURIComponent(values[parameter.name] ?? ""));
  }
  const query = new URLSearchParams();
  for (const parameter of parameters.filter((p) => p.in === "query")) {
    const value = values[parameter.name];
    if (value !== undefined && value !== "") query.set(parameter.name, value);
  }
  const text = query.toString();
  return text ? `${path}?${text}` : path;
}

/** A `curl` command that makes the call with an API key from `$ZUI_KEY`. */
export function curl(method: string, address: string, body: string | null, withKey: boolean): string {
  const parts = [`curl -X ${method.toUpperCase()} '${address}'`];
  if (withKey) parts.push(`-H 'Authorization: Bearer '"$ZUI_KEY"`);
  if (body !== null) {
    parts.push(`-H 'Content-Type: application/json'`);
    parts.push(`-d '${body.replace(/'/g, `'\\''`)}'`);
  }
  return parts.join(" \\\n  ");
}
