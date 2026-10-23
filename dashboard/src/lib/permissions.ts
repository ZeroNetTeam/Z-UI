import { useAuth } from "@/lib/auth";
import type { Admin } from "@/lib/api";

/** The parts of the panel permissions are given for, in the order the server lists them. */
export const sections = [
  "users",
  "groups",
  "templates",
  "inbounds",
  "hosts",
  "certificates",
  "core",
  "monitor",
  "logs",
  "settings",
  "audit",
  "api",
] as const;

export type Section = (typeof sections)[number];
export type Level = "none" | "read" | "write";

/** Nothing there is changed, only looked at. */
export const readOnlySections: Section[] = ["audit"];

/** Whether `admin` may do `permission` (`users.write`); `sudo` asks for a sudo admin. */
export function can(admin: Pick<Admin, "is_sudo" | "permissions"> | null | undefined, permission: string): boolean {
  if (!admin) return false;
  if (permission === "sudo") return admin.is_sudo;
  return admin.permissions.includes(permission);
}

export function useMe(): Admin | null {
  return useAuth().data?.admin ?? null;
}

export function useCan(permission: string): boolean {
  return can(useMe(), permission);
}

/** How much of a section `permissions` give. */
export function levelOf(permissions: string[], section: Section): Level {
  if (permissions.includes(`${section}.write`)) return "write";
  if (permissions.includes(`${section}.read`)) return "read";
  return "none";
}

/** `permissions` with the level of one section replaced. */
export function withLevel(permissions: string[], section: Section, level: Level): string[] {
  const rest = permissions.filter((name) => !name.startsWith(`${section}.`));
  if (level === "none") return rest;
  return [...rest, `${section}.${level}`];
}

/** The most that can be given of every section: a role with it is sudo in all but name. */
export function everything(): string[] {
  return sections.map((section) => `${section}.${readOnlySections.includes(section) ? "read" : "write"}`);
}

/** Only the highest level of each section, in the server's order: what is sent. */
export function minimal(permissions: string[]): string[] {
  return sections.flatMap((section) => {
    const level = levelOf(permissions, section);
    return level === "none" ? [] : [`${section}.${level}`];
  });
}
