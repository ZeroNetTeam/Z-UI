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

export function post<T>(path: string, body?: unknown): Promise<T> {
  return api<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

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

export interface Admin {
  id: number;
  username: string;
  is_sudo: boolean;
  totp_enabled: boolean;
  last_login_at: number | null;
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
