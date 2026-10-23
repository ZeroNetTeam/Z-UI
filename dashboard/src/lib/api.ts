/**
 * The panel's API, relative to the page's <base> so it follows the panel's
 * secret path. Failures carry the server's stable error code, which the
 * dashboard translates (`errors.<code>`).
 */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
    public readonly retryAfter?: number,
  ) {
    super(message);
  }
}

/** Called when the server says the session is gone, to send the user to sign in. */
let onUnauthorized: (() => void) | undefined;
export function setUnauthorizedHandler(handler: () => void) {
  onUnauthorized = handler;
}

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(new URL(`api/${path.replace(/^\//, "")}`, document.baseURI), {
    ...init,
    headers: { Accept: "application/json", ...init?.headers },
    credentials: "same-origin",
  });
  if (!response.ok) {
    let code = "unknown";
    let message = response.statusText;
    let retryAfter: number | undefined;
    try {
      const body = await response.json();
      code = body.error ?? code;
      message = body.message ?? message;
      retryAfter = body.retry_after;
    } catch {
      // Not JSON: keep the status text.
    }
    if (response.status === 401 && code === "unauthorized") onUnauthorized?.();
    throw new ApiError(response.status, code, message, retryAfter);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

function send<T>(method: string, path: string, body?: unknown): Promise<T> {
  return api<T>(path, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

export const post = <T>(path: string, body?: unknown) => send<T>("POST", path, body);
export const put = <T>(path: string, body?: unknown) => send<T>("PUT", path, body);
export const del = <T>(path: string) => api<T>(path, { method: "DELETE" });

export interface NodeStatus {
  running: boolean;
  core_version: string;
  inbounds: string[];
  active_sessions: number;
}

export interface Health {
  product: string;
  version: string;
  core: NodeStatus;
}

export interface RoleRef {
  id: number;
  name: string;
}

/** What an admin has handed out and may still hand out. */
export interface Quota {
  user_limit: number;
  data_quota: number;
  users: number;
  allocated: number;
  unlimited: number;
  used: number;
}

/** The admin using the dashboard, and what they may do. */
export interface Admin {
  id: number;
  username: string;
  is_sudo: boolean;
  totp_enabled: boolean;
  last_login_at: number | null;
  role: RoleRef | null;
  /** Every `<section>.<level>` they may use; a sudo admin has them all. */
  permissions: string[];
  /** They see and change only the users they made. */
  own_users_only: boolean;
  /** The groups they may give users; empty for every group. */
  group_ids: number[];
  quota: Quota;
}

export interface AuthState {
  setup_required: boolean;
  admin: Admin | null;
}

export interface TotpSetup {
  secret: string;
  otpauth: string;
  qr_svg: string;
}

export type Applied = "started" | "reloaded" | "restarted" | "stopped";

export interface Inbound {
  id: number;
  tag: string;
  remark: string;
  enabled: boolean;
  listen: string;
  port: number;
  protocol: string;
  settings: Record<string, unknown>;
  stream: Record<string, unknown>;
  sniffing: Record<string, unknown>;
  created_at: number;
  updated_at: number;
}

export interface InboundPayload {
  tag?: string;
  remark: string;
  enabled: boolean;
  listen: string;
  port: number;
  protocol: string;
  settings: Record<string, unknown>;
  stream: Record<string, unknown>;
  sniffing: Record<string, unknown>;
}

export interface Certificate {
  id: number;
  name: string;
  domains: string[];
  source: string;
  not_before: number;
  not_after: number;
  created_at: number;
}

export interface Host {
  id: number;
  inbound_id: number;
  remark: string;
  address: string;
  port: number | null;
  sni: string;
  host: string;
  path: string;
  alpn: string;
  fingerprint: string;
  allow_insecure: boolean;
  enabled: boolean;
  priority: number;
}

export interface RealityCheck {
  target: string;
  tls13: boolean;
  h2: boolean;
  x25519: boolean;
  certificate_valid: boolean;
  latency_ms: number | null;
  error: string | null;
  suitable: boolean;
}

/** Statuses the panel stores; `limited` and `expired` it sets itself. */
export type UserStatus = "active" | "disabled" | "on_hold" | "limited" | "expired";

export type ResetStrategy = "no_reset" | "day" | "week" | "month" | "year";

export interface User {
  id: number;
  username: string;
  status: UserStatus;
  data_limit: number;
  used_traffic: number;
  lifetime_traffic: number;
  expire_at: number | null;
  on_hold_duration: number | null;
  on_hold_timeout: number | null;
  reset_strategy: ResetStrategy;
  last_reset_at: number | null;
  ip_limit: number;
  speed_up: number;
  speed_down: number;
  note: string;
  online_at: number | null;
  online: boolean;
  group_ids: number[];
  next_plan: NextPlan | null;
  hwid_limit: number;
  sub_updated_at: number | null;
  sub_user_agent: string;
  uuid: string;
  password: string;
  sub_token: string;
  /** The admin who owns the user, and their name. */
  admin_id: number | null;
  owner: string | null;
  created_at: number;
  updated_at: number;
}

export interface UserPayload {
  username: string;
  status: "active" | "disabled" | "on_hold";
  data_limit: number;
  expire_at: number | null;
  on_hold_duration: number | null;
  on_hold_timeout: number | null;
  reset_strategy: ResetStrategy;
  ip_limit: number;
  speed_up: number;
  speed_down: number;
  note: string;
  group_ids: number[];
  next_plan: NextPlan | null;
  hwid_limit: number;
}

/** What a user moves on to when it runs out of data or time. */
export interface NextPlan {
  data_limit: number;
  expire_days: number;
  carry_over: boolean;
}

export interface UserPage {
  users: User[];
  total: number;
}

export type UserCounts = Partial<Record<UserStatus | "total" | "online", number>>;

export interface UsagePoint {
  hour: number;
  uplink: number;
  downlink: number;
}

export type BulkAction =
  | { action: "enable" | "disable" | "delete" | "reset" | "revoke" }
  | { action: "extend"; days: number }
  | { action: "add_data"; bytes: number }
  | { action: "set_groups"; group_ids: number[] };

export interface Connected {
  sessions: number;
  addresses: string[];
}

export interface AccessGroup {
  id: number;
  name: string;
  all_inbounds: boolean;
  inbound_ids: number[];
  users: number;
}

export interface UserTemplate {
  id: number;
  name: string;
  data_limit: number;
  expire_days: number;
  on_hold: boolean;
  reset_strategy: ResetStrategy;
  ip_limit: number;
  speed_up: number;
  speed_down: number;
  hwid_limit: number;
  group_ids: number[];
}

export interface SubscriptionSettings {
  path: string;
  public_url: string;
  server_address: string;
  update_hours: number;
  title: string;
  support_url: string;
  remark_template: string;
  hwid_required: boolean;
  template: string;
}

export interface UserSubscription {
  url: string;
  qr_svg: string;
  links: { remark: string; link: string }[];
}

export interface Device {
  id: number;
  hwid: string;
  platform: string;
  os_version: string;
  model: string;
  user_agent: string;
  first_seen: number;
  last_seen: number;
}

export interface SystemSnapshot {
  at: number;
  cores: number;
  cpu: number | null;
  load: [number, number, number] | null;
  mem_total: number | null;
  mem_used: number | null;
  swap_total: number | null;
  swap_used: number | null;
  disk_total: number | null;
  disk_used: number | null;
  rx_rate: number | null;
  tx_rate: number | null;
  rx_total: number | null;
  tx_total: number | null;
  uptime: number | null;
  panel_rss: number | null;
}

export interface SystemPoint {
  t: number;
  cpu: number | null;
  mem: number | null;
  rx: number | null;
  tx: number | null;
}

export interface SystemView {
  now: SystemSnapshot | null;
  history: SystemPoint[];
  panel_uptime: number;
}

export interface TopUser {
  id: number;
  username: string;
  uplink: number;
  downlink: number;
}

export interface TrafficView {
  points: UsagePoint[];
  top: TopUser[];
  lifetime: number;
}

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export interface LogRecord {
  id: number;
  time: number;
  level: LogLevel;
  source: "core" | "panel" | "other";
  target: string;
  message: string;
}

export interface GeodataStatus {
  geositeTags: number;
  geoipTags: number;
  files: { name: string; outcome: string; detail: string; entries: number; atUnixSeconds: number }[];
}

export interface CoreView {
  settings: Record<string, unknown>;
  defaults: Record<string, unknown>;
  running: boolean;
  inbounds: { tag: string; remark: string; enabled: boolean; protocol: string }[];
  geodata: GeodataStatus | null;
  asset_files: unknown;
}

export interface CoreWarning {
  path: string;
  message: string;
}

export interface IranOptions {
  proxies: string[];
  remote_dns: string;
  local_dns: string;
  anti_sanction_dns: string;
  block_ads: boolean;
  fragment: boolean;
  manage_geodata: boolean;
  clean_ip_candidates: string[];
  clean_ip_host: string;
}

export interface LogLevelView {
  level: string;
  fixed: boolean;
}

/** An admin as a sudo admin sees them. */
export interface AdminRow {
  id: number;
  username: string;
  is_sudo: boolean;
  enabled: boolean;
  role_id: number | null;
  role: string | null;
  totp_enabled: boolean;
  last_login_at: number | null;
  created_at: number;
  user_limit: number;
  data_quota: number;
  group_ids: number[];
  note: string;
  users: number;
  allocated: number;
  unlimited: number;
  used: number;
  keys: number;
}

export interface AdminPayload {
  username?: string;
  password?: string;
  is_sudo: boolean;
  role_id: number | null;
  enabled: boolean;
  user_limit: number;
  data_quota: number;
  group_ids: number[];
  note: string;
}

export interface Role {
  id: number;
  name: string;
  permissions: string[];
  own_users_only: boolean;
  admins: number;
  created_at: number;
}

export interface RoleList {
  catalog: { section: string; writable: boolean }[];
  roles: Role[];
}

export interface RolePayload {
  name: string;
  permissions: string[];
  own_users_only: boolean;
}

export interface ApiKey {
  id: number;
  name: string;
  prefix: string;
  full: boolean;
  permissions: string[];
  allowed_ips: string[];
  expires_at: number | null;
  expired: boolean;
  last_used_at: number | null;
  last_used_ip: string | null;
  created_at: number;
}

/** A key just made: the only time its secret is shown. */
export interface ApiKeyCreated extends ApiKey {
  key: string;
}

export interface ApiKeyPayload {
  name: string;
  permissions: "*" | string[];
  allowed_ips: string[];
  expires_at: number | null;
}

export interface AuditEntry {
  id: number;
  at: number;
  admin_id: number | null;
  admin: string;
  via: string;
  ip: string;
  action: string;
  target: string;
  detail: string;
  status: number;
}

export interface AuditPage {
  entries: AuditEntry[];
  next: number | null;
}
