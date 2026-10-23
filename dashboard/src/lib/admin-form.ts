import type { AdminPayload, AdminRow } from "@/lib/api";
import { bytesToGb, gbToBytes } from "@/lib/users";

/** What the admin dialog edits; numbers are text while they are typed. */
export interface AdminForm {
  username: string;
  password: string;
  isSudo: boolean;
  roleId: string;
  enabled: boolean;
  userLimit: string;
  dataQuotaGb: string;
  groupIds: number[];
  note: string;
}

export function emptyAdmin(roleId?: number): AdminForm {
  return {
    username: "",
    password: "",
    isSudo: false,
    roleId: roleId === undefined ? "" : String(roleId),
    enabled: true,
    userLimit: "0",
    dataQuotaGb: "0",
    groupIds: [],
    note: "",
  };
}

export function fromAdmin(admin: AdminRow): AdminForm {
  return {
    username: admin.username,
    password: "",
    isSudo: admin.is_sudo,
    roleId: admin.role_id === null ? "" : String(admin.role_id),
    enabled: admin.enabled,
    userLimit: String(admin.user_limit),
    dataQuotaGb: bytesToGb(admin.data_quota),
    groupIds: admin.group_ids,
    note: admin.note,
  };
}

/**
 * What the server is sent. A sudo admin has no role, quota or group limit
 * (those are for admins held to something); a new admin names itself and
 * brings a password, an old one sends a password only to change it.
 */
export function toAdminPayload(form: AdminForm, creating: boolean): AdminPayload {
  const sudo = form.isSudo;
  const payload: AdminPayload = {
    is_sudo: sudo,
    role_id: sudo || form.roleId === "" ? null : Number(form.roleId),
    enabled: form.enabled,
    user_limit: sudo ? 0 : Math.max(0, Math.round(Number(form.userLimit) || 0)),
    data_quota: sudo ? 0 : gbToBytes(form.dataQuotaGb),
    group_ids: sudo ? [] : form.groupIds,
    note: form.note.trim(),
  };
  if (creating) payload.username = form.username.trim();
  if (creating || form.password !== "") payload.password = form.password;
  return payload;
}

/** The share of `quota` that `allocated` is, 0 to 100; none when there is no quota. */
export function quotaPercent(allocated: number, quota: number): number | null {
  if (quota <= 0) return null;
  return Math.min(100, Math.round((allocated / quota) * 100));
}
