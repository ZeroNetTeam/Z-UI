/**
 * The Core page's forms, as plain data, and their translation to and from
 * the core's JSON (Xray's shape). Each `to…` starts from what was there
 * before, so a key the form does not know about (a rule's `attrs`, an
 * outbound's `mux`) survives an edit instead of being dropped.
 */

export type Json = Record<string, unknown>;

const obj = (value: unknown): Json => (value && typeof value === "object" && !Array.isArray(value) ? (value as Json) : {});
const str = (value: unknown, fallback = "") => (typeof value === "string" ? value : fallback);
export const strings = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : typeof value === "string" ? [value] : [];
const clone = <T>(value: T): T => structuredClone(value);

/** `value` as JSON with every object's keys in order, so two documents with
 * the same content compare equal whatever order their keys came in. */
export function canonical(value: unknown): string {
  const sort = (item: unknown): unknown =>
    Array.isArray(item)
      ? item.map(sort)
      : item && typeof item === "object"
        ? Object.fromEntries(Object.entries(item as Json).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([key, v]) => [key, sort(v)]))
        : item;
  return JSON.stringify(sort(value));
}

/** One entry per line or comma, blanks dropped. */
export const lines = (text: string): string[] =>
  text
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);

/** A list back into the text a textarea holds. */
export const text = (list: unknown): string => strings(list).join("\n");

/** `obj` without `key`s that are empty (an empty object, list or text). */
function prune(target: Json, ...keys: string[]) {
  for (const key of keys) {
    const value = target[key];
    const empty =
      value === undefined ||
      value === null ||
      value === "" ||
      (Array.isArray(value) && value.length === 0) ||
      (typeof value === "object" && !Array.isArray(value) && Object.keys(value as Json).length === 0);
    if (empty) delete target[key];
  }
}

// ---- Outbounds

export const outboundKinds = ["link", "freedom", "blackhole", "tor", "json"] as const;
export type OutboundKind = (typeof outboundKinds)[number];
export const domainStrategies = ["AsIs", "UseIP", "UseIPv4", "UseIPv6"] as const;

/** What splitting a TLS hello looks like to the core. */
export const FRAGMENT = { packets: "1-1", length: "40-80", interval: "1-1" };

export interface OutboundForm {
  kind: OutboundKind;
  tag: string;
  link: string;
  /** The tag of the outbound that carries this one to its server, if any. */
  dialThrough: string;
  fragment: boolean;
  allowInsecure: boolean;
  domainStrategy: string;
  /** The whole outbound, for the JSON kind. */
  json: string;
}

export function outboundKind(outbound: Json): OutboundKind {
  if (typeof outbound.link === "string") return "link";
  switch (outbound.protocol) {
    case "freedom":
      return "freedom";
    case "blackhole":
      return "blackhole";
    case "tor":
      return "tor";
    default:
      return "json";
  }
}

export function emptyOutbound(kind: OutboundKind = "link"): OutboundForm {
  return { kind, tag: "", link: "", dialThrough: "", fragment: false, allowInsecure: false, domainStrategy: "AsIs", json: "{\n  \n}" };
}

export function fromOutbound(outbound: Json): OutboundForm {
  const kind = outboundKind(outbound);
  return {
    kind,
    tag: str(outbound.tag),
    link: str(outbound.link),
    dialThrough: str(obj(obj(outbound.streamSettings).sockopt).dialerProxy),
    fragment: Object.keys(obj(obj(outbound.evasion).fragment)).length > 0,
    allowInsecure: outbound.allowInsecure === true,
    domainStrategy: str(obj(outbound.settings).domainStrategy, "AsIs"),
    json: JSON.stringify(outbound, null, 2),
  };
}

/** The outbound a form describes; throws a message when the JSON kind is not an object. */
export function toOutbound(form: OutboundForm, previous?: Json): Json {
  const same = previous && outboundKind(previous) === form.kind ? clone(previous) : {};
  const tag = form.tag.trim();
  switch (form.kind) {
    case "link": {
      const out: Json = { ...same, tag, link: form.link.trim() };
      const stream = obj(out.streamSettings);
      const sockopt = obj(stream.sockopt);
      if (form.dialThrough) sockopt.dialerProxy = form.dialThrough;
      else delete sockopt.dialerProxy;
      stream.sockopt = sockopt;
      out.streamSettings = stream;
      const evasion = obj(out.evasion);
      if (form.fragment) evasion.fragment = obj(evasion.fragment).packets ? evasion.fragment : { ...FRAGMENT };
      else delete evasion.fragment;
      out.evasion = evasion;
      if (form.allowInsecure) out.allowInsecure = true;
      else delete out.allowInsecure;
      prune(stream, "sockopt");
      prune(out, "streamSettings", "evasion");
      return out;
    }
    case "freedom": {
      const out: Json = { ...same, tag, protocol: "freedom" };
      const settings = obj(out.settings);
      if (form.domainStrategy && form.domainStrategy !== "AsIs") settings.domainStrategy = form.domainStrategy;
      else delete settings.domainStrategy;
      out.settings = settings;
      prune(out, "settings");
      return out;
    }
    case "blackhole":
      return { ...same, tag, protocol: "blackhole" };
    case "tor":
      return { ...same, tag, protocol: "tor", settings: obj(same.settings) };
    case "json": {
      let parsed: unknown;
      try {
        parsed = JSON.parse(form.json);
      } catch (error) {
        throw new Error((error as Error).message);
      }
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("not an object");
      const out = parsed as Json;
      if (tag) out.tag = tag;
      return out;
    }
  }
}

/** `vless · host:443` for a link, the protocol and first server for the rest. */
export function outboundSummary(outbound: Json): string {
  if (typeof outbound.link === "string") {
    const link = outbound.link;
    const scheme = link.split("://")[0]?.toLowerCase() ?? "";
    try {
      const url = new URL(link);
      const host = url.hostname ? `${url.hostname}${url.port ? `:${url.port}` : ""}` : "";
      return host ? `${scheme} · ${host}` : scheme;
    } catch {
      return scheme;
    }
  }
  const protocol = str(outbound.protocol, "?");
  const settings = obj(outbound.settings);
  const server = obj((Array.isArray(settings.vnext) ? settings.vnext : Array.isArray(settings.servers) ? settings.servers : [])[0]);
  const address = str(server.address) || str(settings.address);
  const port = server.port ?? settings.port;
  return address ? `${protocol} · ${address}${port ? `:${String(port)}` : ""}` : protocol;
}

/** Where `tag` is named by something else, so deleting it would break that. */
export function usedBy(settings: Json, tag: string): string[] {
  const found: string[] = [];
  const routing = obj(settings.routing);
  (Array.isArray(routing.rules) ? routing.rules : []).forEach((rule, index) => {
    if (obj(rule).outboundTag === tag) found.push(`routing.rules[${index}]`);
  });
  (Array.isArray(routing.balancers) ? routing.balancers : []).forEach((balancer) => {
    if (strings(obj(balancer).selector).some((prefix) => tag.startsWith(prefix))) found.push(`balancer ${str(obj(balancer).tag)}`);
    if (obj(balancer).fallbackTag === tag) found.push(`balancer ${str(obj(balancer).tag)}`);
  });
  (Array.isArray(settings.outbounds) ? settings.outbounds : []).forEach((outbound) => {
    const o = obj(outbound);
    if (str(o.tag) !== tag && str(obj(obj(o.streamSettings).sockopt).dialerProxy) === tag) found.push(`outbound ${str(o.tag)}`);
  });
  strings(obj(settings.observatory).subjectSelector).forEach((prefix) => {
    if (tag.startsWith(prefix)) found.push("observatory");
  });
  return [...new Set(found)];
}

/**
 * Whether any outbound sends traffic through another server. Then the core
 * refuses this machine's own resolver whatever `leakPolicy` says.
 */
export function hasProxyOutbound(settings: Json): boolean {
  return (Array.isArray(settings.outbounds) ? settings.outbounds : []).some((item) => {
    const outbound = obj(item);
    if (typeof outbound.link === "string") return !/^(direct|socks|masterdns)/i.test(outbound.link);
    return !["freedom", "blackhole", "dns"].includes(str(outbound.protocol));
  });
}

/** A resolver that is this machine's own (`localhost`), which strict DNS refuses. */
export const isSystemResolver = (server: unknown): boolean => {
  const address = typeof server === "string" ? server : str(obj(server).address);
  return address.trim().toLowerCase() === "localhost";
};

// ---- Fragment (an outbound's TLS hello split in pieces)

const FRAGMENT_MASK = { tcp: [{ type: "fragment", settings: { packets: "1-1", length: "40-80", interval: "1-1" } }] };

/** Whether an outbound carrying TLS or REALITY splits its hello. */
export function hasFragment(outbound: Json): boolean {
  if (typeof outbound.link === "string") return Object.keys(obj(obj(outbound.evasion).fragment)).length > 0;
  const mask = obj(obj(outbound.streamSettings).finalmask);
  return Array.isArray(mask.tcp) && mask.tcp.some((entry) => obj(entry).type === "fragment");
}

/** `outbound` with its hello split, or not. */
export function withFragment(outbound: Json, on: boolean): Json {
  const out = clone(outbound);
  if (typeof out.link === "string") {
    const evasion = obj(out.evasion);
    if (on) evasion.fragment = Object.keys(obj(evasion.fragment)).length ? evasion.fragment : { ...FRAGMENT };
    else delete evasion.fragment;
    out.evasion = evasion;
    prune(out, "evasion");
    return out;
  }
  const stream = obj(out.streamSettings);
  const mask = obj(stream.finalmask);
  const others = (Array.isArray(mask.tcp) ? mask.tcp : []).filter((entry) => obj(entry).type !== "fragment");
  mask.tcp = on ? [...clone(FRAGMENT_MASK.tcp), ...others] : others;
  stream.finalmask = mask;
  out.streamSettings = stream;
  prune(mask, "tcp");
  prune(stream, "finalmask");
  prune(out, "streamSettings");
  return out;
}

/** Whether fragmenting makes sense for an outbound: it must carry a hello to split. */
export function canFragment(outbound: Json): boolean {
  if (typeof outbound.link === "string") return !/^(warp|direct|tor|socks|masterdns)/i.test(outbound.link);
  const protocol = str(outbound.protocol);
  if (["freedom", "blackhole", "tor", "dns", "warp", "masque", "wireguard", "hysteria2", "tuic", "juicity"].includes(protocol)) return false;
  const security = str(obj(outbound.streamSettings).security);
  return security === "tls" || security === "reality";
}

// ---- Routing rules

export interface RuleForm {
  inboundTag: string[];
  domain: string;
  ip: string;
  port: string;
  /** `tcp`, `udp`, `tcp,udp`, or blank for both. */
  network: string;
  protocol: string[];
  user: string;
  targetKind: "outbound" | "balancer";
  target: string;
}

export const sniffedProtocols = ["http", "tls", "quic", "bittorrent"] as const;

export function emptyRule(target = ""): RuleForm {
  return { inboundTag: [], domain: "", ip: "", port: "", network: "", protocol: [], user: "", targetKind: "outbound", target };
}

export function fromRule(rule: Json): RuleForm {
  const balancer = str(rule.balancerTag);
  return {
    inboundTag: strings(rule.inboundTag),
    domain: text(rule.domain),
    ip: text(rule.ip),
    port: str(rule.port) || (typeof rule.port === "number" ? String(rule.port) : ""),
    network: str(rule.network),
    protocol: strings(rule.protocol),
    user: text(rule.user),
    targetKind: balancer ? "balancer" : "outbound",
    target: balancer || str(rule.outboundTag),
  };
}

export function toRule(form: RuleForm, previous?: Json): Json {
  const rule: Json = { ...(previous ? clone(previous) : {}), type: "field" };
  const set = (key: string, value: string[] | string) => {
    if (Array.isArray(value) ? value.length > 0 : value.trim() !== "") rule[key] = Array.isArray(value) ? value : value.trim();
    else delete rule[key];
  };
  set("inboundTag", form.inboundTag);
  set("domain", lines(form.domain));
  set("ip", lines(form.ip));
  set("port", form.port);
  set("network", form.network);
  set("protocol", form.protocol);
  set("user", lines(form.user));
  delete rule.outboundTag;
  delete rule.balancerTag;
  rule[form.targetKind === "balancer" ? "balancerTag" : "outboundTag"] = form.target;
  return rule;
}

/** The conditions of a rule in a few words, `domain: geosite:ir · port: 443`. */
export function ruleConditions(rule: Json): string[] {
  const parts: string[] = [];
  const add = (label: string, value: unknown) => {
    const list = strings(value).length ? strings(value) : typeof value === "number" ? [String(value)] : [];
    if (list.length) parts.push(`${label}: ${list.length > 2 ? `${list.slice(0, 2).join(", ")} +${list.length - 2}` : list.join(", ")}`);
  };
  add("inbound", rule.inboundTag);
  add("domain", rule.domain);
  add("ip", rule.ip);
  add("port", rule.port);
  add("network", rule.network);
  add("protocol", rule.protocol);
  add("user", rule.user);
  return parts;
}

export const ruleTarget = (rule: Json): string => str(rule.balancerTag) || str(rule.outboundTag);

/**
 * Whether the settings name a geosite or geoip list (`geoip:private` is built
 * in and needs none), which only match once the lists are loaded.
 */
export function usesGeodata(settings: Json): boolean {
  return /geo(?:site|ip):(?!private\b)[^"\s,]+/.test(JSON.stringify({ ...settings, assets: undefined }));
}

// ---- Balancers and the observatory

export const balancerStrategies = ["random", "roundRobin", "leastPing", "leastLoad"] as const;

export interface BalancerForm {
  tag: string;
  selector: string;
  strategy: string;
  fallbackTag: string;
}

export function emptyBalancer(): BalancerForm {
  return { tag: "", selector: "", strategy: "random", fallbackTag: "" };
}

export function fromBalancer(balancer: Json): BalancerForm {
  return {
    tag: str(balancer.tag),
    selector: text(balancer.selector),
    strategy: str(obj(balancer.strategy).type, "random"),
    fallbackTag: str(balancer.fallbackTag),
  };
}

export function toBalancer(form: BalancerForm, previous?: Json): Json {
  const out: Json = { ...(previous ? clone(previous) : {}), tag: form.tag.trim(), selector: lines(form.selector) };
  out.strategy = { ...obj(out.strategy), type: form.strategy };
  if (form.fallbackTag) out.fallbackTag = form.fallbackTag;
  else delete out.fallbackTag;
  return out;
}

/**
 * The observatory with the clean-IP search switched on (`candidates` and
 * `host` set), or off. The search needs an observatory around it, so turning
 * it on makes one; turning it off takes away an observatory that holds
 * nothing else, rather than leaving an empty one behind.
 */
export function withCleanIp(observatory: unknown, on: boolean, candidates: string[], host: string): Json | undefined {
  const out: Json = { ...obj(observatory) };
  if (!on) {
    delete out.cleanIp;
    const measuring = strings(out.subjectSelector).length > 0;
    const others = Object.keys(out).filter((key) => !["probeUrl", "probeInterval", "subjectSelector"].includes(key));
    return measuring || others.length > 0 ? out : undefined;
  }
  out.probeUrl = str(out.probeUrl) || "https://www.gstatic.com/generate_204";
  out.probeInterval = str(out.probeInterval) || "300s";
  out.subjectSelector = strings(out.subjectSelector);
  out.cleanIp = { ...obj(out.cleanIp), candidates, host: host.trim() || "www.speedtest.net", path: "/" };
  return out;
}

export interface ObservatoryForm {
  enabled: boolean;
  subjectSelector: string;
  probeUrl: string;
  probeInterval: string;
}

export function fromObservatory(observatory: unknown): ObservatoryForm {
  const o = obj(observatory);
  return {
    enabled: Object.keys(o).length > 0,
    subjectSelector: text(o.subjectSelector),
    probeUrl: str(o.probeUrl, "https://www.gstatic.com/generate_204"),
    probeInterval: str(o.probeInterval, "300s"),
  };
}

/** The observatory object, keeping what the form does not edit (clean-IP). */
export function toObservatory(form: ObservatoryForm, previous?: unknown): Json | undefined {
  if (!form.enabled) return undefined;
  return { ...obj(previous), subjectSelector: lines(form.subjectSelector), probeUrl: form.probeUrl.trim(), probeInterval: form.probeInterval.trim() };
}

// ---- DNS

export const queryStrategies = ["UseIP", "UseIPv4", "UseIPv6"] as const;

export interface DnsServerForm {
  address: string;
  domains: string;
  expectIPs: string;
  tag: string;
  skipFallback: boolean;
}

export function emptyDnsServer(): DnsServerForm {
  return { address: "", domains: "", expectIPs: "", tag: "", skipFallback: false };
}

export function fromDnsServer(server: unknown): DnsServerForm {
  if (typeof server === "string") return { ...emptyDnsServer(), address: server };
  const o = obj(server);
  return {
    address: str(o.address),
    domains: text(o.domains),
    expectIPs: text(o.expectedIPs ?? o.expectIPs),
    tag: str(o.tag),
    skipFallback: o.skipFallback === true,
  };
}

/** A bare string while the server is only an address, an object when it says more. */
export function toDnsServer(form: DnsServerForm, previous?: unknown): unknown {
  const address = form.address.trim();
  const domains = lines(form.domains);
  const expected = lines(form.expectIPs);
  const tag = form.tag.trim();
  const plain = typeof previous === "string" || previous === undefined;
  if (plain && domains.length === 0 && expected.length === 0 && !tag && !form.skipFallback) return address;
  const out: Json = { ...obj(typeof previous === "object" ? previous : undefined), address };
  const setList = (key: string, list: string[]) => {
    if (list.length) out[key] = list;
    else delete out[key];
  };
  setList("domains", domains);
  delete out.expectIPs;
  setList("expectedIPs", expected);
  if (tag) out.tag = tag;
  else delete out.tag;
  if (form.skipFallback) out.skipFallback = true;
  else delete out.skipFallback;
  return out;
}

export function dnsServerSummary(server: unknown): string {
  const form = fromDnsServer(server);
  const scope = lines(form.domains);
  return scope.length ? `${form.address} → ${scope.length > 2 ? `${scope.slice(0, 2).join(", ")} +${scope.length - 2}` : scope.join(", ")}` : form.address;
}

/** `hosts` as the lines `domain value` (a value of `#…` blocks the name). */
export function hostsText(hosts: unknown): string {
  return Object.entries(obj(hosts))
    .map(([name, value]) => `${name} ${Array.isArray(value) ? value.join(",") : String(value)}`)
    .join("\n");
}

export function parseHosts(input: string): Json {
  const hosts: Json = {};
  for (const line of input.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("//")) continue;
    const [name, ...rest] = trimmed.split(/\s+/);
    const value = rest.join("").split(",").filter(Boolean);
    if (name && value.length) hosts[name] = value.length === 1 ? value[0] : value;
  }
  return hosts;
}
