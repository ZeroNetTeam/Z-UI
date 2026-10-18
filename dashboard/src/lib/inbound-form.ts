/**
 * The inbound form, as plain data, and its translation to and from what the
 * panel stores (Xray's shape). Kept free of React so it can be tested.
 */
import type { Inbound, InboundPayload } from "./api";

export const protocols = [
  "vless",
  "vmess",
  "trojan",
  "shadowsocks",
  "hysteria2",
  "tuic",
  "anytls",
  "wireguard",
] as const;
export type Protocol = (typeof protocols)[number];

export const transports = ["raw", "ws", "grpc", "httpupgrade", "xhttp"] as const;
export type Transport = (typeof transports)[number];
export type Security = "none" | "tls" | "reality";

/** What each protocol can be carried over and secured with. */
export const capabilities: Record<Protocol, { transports: readonly Transport[]; securities: Security[] }> = {
  vless: { transports, securities: ["reality", "tls", "none"] },
  vmess: { transports, securities: ["none", "tls"] },
  trojan: { transports, securities: ["tls", "reality", "none"] },
  shadowsocks: { transports: ["raw"], securities: ["none"] },
  hysteria2: { transports: [], securities: ["tls"] },
  tuic: { transports: [], securities: ["tls"] },
  anytls: { transports: ["raw"], securities: ["tls"] },
  wireguard: { transports: [], securities: ["none"] },
};

export const shadowsocksMethods = [
  "2022-blake3-aes-128-gcm",
  "2022-blake3-aes-256-gcm",
  "aes-128-gcm",
  "aes-256-gcm",
  "chacha20-ietf-poly1305",
] as const;

/** Key bytes a Shadowsocks 2022 method needs; 0 for the legacy ones. */
export function shadowsocksKeyBytes(method: string): number {
  if (method === "2022-blake3-aes-128-gcm") return 16;
  if (method.startsWith("2022-")) return 32;
  return 0;
}

export const fingerprints = ["chrome", "firefox", "safari", "ios", "android", "edge", "random"] as const;
export const xhttpModes = ["auto", "packet-up", "stream-up", "stream-one"] as const;

export interface InboundForm {
  remark: string;
  enabled: boolean;
  listen: string;
  port: number;
  protocol: Protocol;
  transport: Transport;
  path: string;
  host: string;
  serviceName: string;
  xhttpMode: (typeof xhttpModes)[number];
  security: Security;
  certificateId: number | null;
  alpn: string;
  realityTarget: string;
  realityServerNames: string;
  realityPrivateKey: string;
  realityPublicKey: string;
  realityShortIds: string;
  fingerprint: string;
  flow: "" | "xtls-rprx-vision";
  ssMethod: string;
  ssPassword: string;
  obfsPassword: string;
  wgSecretKey: string;
  wgPublicKey: string;
  wgMtu: number;
  sniffing: boolean;
}

export function randomPort(): number {
  return 10000 + Math.floor(Math.random() * 50000);
}

export function defaults(protocol: Protocol = "vless"): InboundForm {
  const security = capabilities[protocol].securities[0];
  return {
    remark: "",
    enabled: true,
    listen: "",
    port: randomPort(),
    protocol,
    transport: capabilities[protocol].transports[0] ?? "raw",
    path: "/",
    host: "",
    serviceName: "grpc",
    xhttpMode: "auto",
    security,
    certificateId: null,
    alpn: "h2,http/1.1",
    realityTarget: "www.microsoft.com:443",
    realityServerNames: "www.microsoft.com",
    realityPrivateKey: "",
    realityPublicKey: "",
    realityShortIds: "",
    fingerprint: "chrome",
    flow: protocol === "vless" ? "xtls-rprx-vision" : "",
    ssMethod: "2022-blake3-aes-128-gcm",
    ssPassword: "",
    obfsPassword: "",
    wgSecretKey: "",
    wgPublicKey: "",
    wgMtu: 1420,
    sniffing: true,
  };
}

const list = (text: string) =>
  text
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean);

/** Whether Vision can be used: VLESS straight over TCP with TLS or REALITY. */
export function visionPossible(form: InboundForm): boolean {
  return form.protocol === "vless" && form.transport === "raw" && form.security !== "none";
}

export function toPayload(form: InboundForm): InboundPayload {
  const caps = capabilities[form.protocol];
  const settings: Record<string, unknown> = {};
  const stream: Record<string, unknown> = {};

  switch (form.protocol) {
    case "vless":
      settings.decryption = "none";
      if (form.flow && visionPossible(form)) settings.flow = form.flow;
      break;
    case "shadowsocks":
      settings.method = form.ssMethod;
      if (shadowsocksKeyBytes(form.ssMethod) > 0) settings.password = form.ssPassword;
      break;
    case "hysteria2":
      if (form.obfsPassword) settings.obfs = { type: "salamander", password: form.obfsPassword };
      break;
    case "wireguard":
      settings.secretKey = form.wgSecretKey;
      settings.mtu = form.wgMtu;
      settings.publicKey = form.wgPublicKey;
      break;
  }

  if (caps.transports.length > 0) {
    stream.network = form.transport;
    if (form.transport === "ws") {
      stream.wsSettings = { path: form.path || "/", ...(form.host ? { headers: { Host: form.host } } : {}) };
    } else if (form.transport === "httpupgrade") {
      stream.httpupgradeSettings = { path: form.path || "/", ...(form.host ? { host: form.host } : {}) };
    } else if (form.transport === "grpc") {
      stream.grpcSettings = { serviceName: form.serviceName };
    } else if (form.transport === "xhttp") {
      stream.xhttpSettings = { path: form.path || "/", mode: form.xhttpMode, ...(form.host ? { host: form.host } : {}) };
    }
  }

  const security = caps.securities.includes(form.security) ? form.security : caps.securities[0];
  if (security !== "none" || caps.transports.length > 0) stream.security = security;
  if (security === "tls") {
    stream.tlsSettings = { certificateId: form.certificateId, alpn: list(form.alpn) };
  } else if (security === "reality") {
    stream.realitySettings = {
      target: form.realityTarget,
      serverNames: list(form.realityServerNames),
      privateKey: form.realityPrivateKey,
      publicKey: form.realityPublicKey,
      shortIds: list(form.realityShortIds),
      fingerprint: form.fingerprint,
    };
  }

  return {
    remark: form.remark.trim(),
    enabled: form.enabled,
    listen: form.listen.trim(),
    port: form.port,
    protocol: form.protocol,
    settings,
    stream,
    sniffing: form.sniffing ? { enabled: true, destOverride: ["http", "tls", "quic"] } : { enabled: false },
  };
}

type Json = Record<string, unknown>;
const obj = (value: unknown): Json => (value && typeof value === "object" ? (value as Json) : {});
const str = (value: unknown, fallback = "") => (typeof value === "string" ? value : fallback);
const strings = (value: unknown) => (Array.isArray(value) ? value.filter((v) => typeof v === "string") : []);

export function fromInbound(inbound: Inbound): InboundForm {
  const protocol = (protocols as readonly string[]).includes(inbound.protocol)
    ? (inbound.protocol as Protocol)
    : "vless";
  const form = defaults(protocol);
  const settings = obj(inbound.settings);
  const stream = obj(inbound.stream);
  form.remark = inbound.remark;
  form.enabled = inbound.enabled;
  form.listen = inbound.listen;
  form.port = inbound.port;
  form.flow = settings.flow === "xtls-rprx-vision" ? "xtls-rprx-vision" : "";
  form.ssMethod = str(settings.method, form.ssMethod);
  form.ssPassword = str(settings.password);
  form.obfsPassword = str(obj(settings.obfs).password);
  form.wgSecretKey = str(settings.secretKey);
  form.wgPublicKey = str(settings.publicKey);
  form.wgMtu = typeof settings.mtu === "number" ? settings.mtu : form.wgMtu;

  const network = str(stream.network, "raw") === "tcp" ? "raw" : str(stream.network, "raw");
  if ((transports as readonly string[]).includes(network)) form.transport = network as Transport;
  const ws = obj(stream.wsSettings);
  const upgrade = obj(stream.httpupgradeSettings);
  const xhttp = obj(stream.xhttpSettings);
  form.path = str(ws.path) || str(upgrade.path) || str(xhttp.path) || "/";
  form.host = str(obj(ws.headers).Host) || str(upgrade.host) || str(xhttp.host);
  form.serviceName = str(obj(stream.grpcSettings).serviceName, form.serviceName);
  const mode = str(xhttp.mode, "auto");
  form.xhttpMode = (xhttpModes as readonly string[]).includes(mode) ? (mode as InboundForm["xhttpMode"]) : "auto";

  const security = str(stream.security, capabilities[protocol].securities[0]);
  form.security = ["none", "tls", "reality"].includes(security) ? (security as Security) : form.security;
  const tls = obj(stream.tlsSettings);
  form.certificateId = typeof tls.certificateId === "number" ? tls.certificateId : null;
  if (Array.isArray(tls.alpn)) form.alpn = strings(tls.alpn).join(",");
  const reality = obj(stream.realitySettings);
  form.realityTarget = str(reality.target) || str(reality.dest) || form.realityTarget;
  form.realityServerNames = strings(reality.serverNames).join(", ") || form.realityServerNames;
  form.realityPrivateKey = str(reality.privateKey);
  form.realityPublicKey = str(reality.publicKey);
  form.realityShortIds = strings(reality.shortIds).join(", ");
  form.fingerprint = str(reality.fingerprint, form.fingerprint);
  form.sniffing = obj(inbound.sniffing).enabled === true;
  return form;
}

/** A short description for the inbounds table: `ws · tls`. */
export function summary(inbound: Inbound): string {
  const stream = obj(inbound.stream);
  const caps = capabilities[inbound.protocol as Protocol];
  const parts: string[] = [];
  if ((caps?.transports.length ?? 0) > 1) parts.push(str(stream.network, "raw"));
  const security = str(stream.security);
  if (security && security !== "none") parts.push(security);
  return parts.join(" · ") || "—";
}
