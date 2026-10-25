import type { MigrateOptions, MigrateWarning } from "@/lib/api";

/** What an import brings unless the admin says otherwise. */
export const defaultOptions: MigrateOptions = {
  inbounds: true,
  users: true,
  hosts: true,
  templates: true,
  admins: true,
  usage: true,
  settings: true,
  // The old panel still holds the ports until it is stopped.
  enable_inbounds: false,
  on_conflict: "skip",
};

export interface WarningGroup {
  code: string;
  scope: string;
  items: MigrateWarning[];
}

/**
 * The warnings gathered by what is the matter, the commonest first, so a
 * hundred users with the same trouble read as one line.
 */
export function groupWarnings(warnings: MigrateWarning[]): WarningGroup[] {
  const groups = new Map<string, WarningGroup>();
  for (const warning of warnings) {
    const key = `${warning.scope}/${warning.code}`;
    const group = groups.get(key) ?? { code: warning.code, scope: warning.scope, items: [] };
    group.items.push(warning);
    groups.set(key, group);
  }
  return [...groups.values()].sort((a, b) => b.items.length - a.items.length || a.code.localeCompare(b.code) || a.scope.localeCompare(b.scope));
}

/** The names a group of warnings is about, the first few. */
export function namesOf(group: WarningGroup, limit = 8): { shown: string[]; more: number } {
  const names = [...new Set(group.items.map((item) => item.name).filter((name) => name !== ""))];
  return { shown: names.slice(0, limit), more: Math.max(0, names.length - limit) };
}

/** Only a file that can be a database is offered the upload. */
export function looksLikeDatabase(name: string): boolean {
  return !/\.(json|txt|zip|gz|tar|sql)$/i.test(name);
}
