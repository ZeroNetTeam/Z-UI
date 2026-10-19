import { describe, expect, it } from "vitest";
import {
  canFragment,
  canonical,
  emptyDnsServer,
  emptyOutbound,
  emptyRule,
  fromBalancer,
  fromDnsServer,
  fromObservatory,
  fromOutbound,
  fromRule,
  hasFragment,
  hasProxyOutbound,
  isSystemResolver,
  hostsText,
  lines,
  outboundKind,
  outboundSummary,
  parseHosts,
  ruleConditions,
  toBalancer,
  toDnsServer,
  toObservatory,
  toOutbound,
  toRule,
  usedBy,
  withCleanIp,
  usesGeodata,
  withFragment,
  type Json,
} from "./core-form";

const link = "vless://5783a3e7-e373-51cd-8642-c83782b807c5@203.0.113.9:443?security=reality#out";

describe("outbounds", () => {
  it("tells the kinds apart", () => {
    expect(outboundKind({ tag: "a", link })).toBe("link");
    expect(outboundKind({ protocol: "freedom" })).toBe("freedom");
    expect(outboundKind({ protocol: "blackhole" })).toBe("blackhole");
    expect(outboundKind({ protocol: "tor" })).toBe("tor");
    expect(outboundKind({ protocol: "vless", settings: {} })).toBe("json");
  });

  it("round-trips a link and keeps what the form does not show", () => {
    const stored: Json = { tag: "proxy", link, mux: { enabled: true }, streamSettings: { sockopt: { tcpCongestion: "bbr" } } };
    const form = fromOutbound(stored);
    expect(form).toMatchObject({ kind: "link", tag: "proxy", link, dialThrough: "", fragment: false });
    expect(toOutbound(form, stored)).toEqual(stored);

    const changed = toOutbound({ ...form, dialThrough: "warp", fragment: true, allowInsecure: true }, stored);
    expect(changed).toMatchObject({
      mux: { enabled: true },
      allowInsecure: true,
      evasion: { fragment: { packets: "1-1" } },
      streamSettings: { sockopt: { tcpCongestion: "bbr", dialerProxy: "warp" } },
    });
    // Switching them off leaves no empty husks behind.
    const bare = toOutbound({ ...fromOutbound(changed), dialThrough: "", fragment: false, allowInsecure: false }, changed);
    expect(bare).toEqual(stored);
    expect(toOutbound({ ...emptyOutbound("link"), tag: "x", link: ` ${link} ` })).toEqual({ tag: "x", link });
  });

  it("writes freedom, blackhole and tor, and reads the JSON kind", () => {
    expect(toOutbound({ ...emptyOutbound("freedom"), tag: "direct" })).toEqual({ tag: "direct", protocol: "freedom" });
    expect(toOutbound({ ...emptyOutbound("freedom"), tag: "d", domainStrategy: "UseIPv4" })).toEqual({
      tag: "d",
      protocol: "freedom",
      settings: { domainStrategy: "UseIPv4" },
    });
    expect(toOutbound({ ...emptyOutbound("blackhole"), tag: "block" })).toEqual({ tag: "block", protocol: "blackhole" });
    expect(toOutbound({ ...emptyOutbound("tor"), tag: "tor" })).toEqual({ tag: "tor", protocol: "tor", settings: {} });
    const json = toOutbound({ ...emptyOutbound("json"), tag: "mine", json: '{"protocol":"trojan","settings":{}}' });
    expect(json).toEqual({ protocol: "trojan", settings: {}, tag: "mine" });
    expect(() => toOutbound({ ...emptyOutbound("json"), json: "[1]" })).toThrow("not an object");
    expect(() => toOutbound({ ...emptyOutbound("json"), json: "{nope" })).toThrow();
  });

  it("summarises what an outbound reaches", () => {
    expect(outboundSummary({ link })).toBe("vless · 203.0.113.9:443");
    expect(outboundSummary({ link: "vless://u@host.example:8443?x=1" })).toBe("vless · host.example:8443");
    expect(outboundSummary({ protocol: "freedom" })).toBe("freedom");
    expect(outboundSummary({ protocol: "trojan", settings: { servers: [{ address: "a.example", port: 443 }] } })).toBe("trojan · a.example:443");
  });

  it("finds what names an outbound", () => {
    const settings: Json = {
      outbounds: [{ tag: "warp", link: "warp://x" }, { tag: "proxy", streamSettings: { sockopt: { dialerProxy: "warp" } } }],
      routing: {
        rules: [{ outboundTag: "block" }, { outboundTag: "warp" }],
        balancers: [{ tag: "b", selector: ["pro"] }],
      },
      observatory: { subjectSelector: ["warp"] },
    };
    expect(usedBy(settings, "warp")).toEqual(["routing.rules[1]", "outbound proxy", "observatory"]);
    expect(usedBy(settings, "proxy")).toEqual(["balancer b"]);
    expect(usedBy(settings, "unused")).toEqual([]);
  });
});

describe("fragment", () => {
  it("is layered on a link and written as a mask on a full outbound", () => {
    const linked: Json = { tag: "p", link };
    expect(hasFragment(linked)).toBe(false);
    const on = withFragment(linked, true);
    expect(hasFragment(on)).toBe(true);
    expect(withFragment(on, false)).toEqual(linked);

    const full: Json = { tag: "p", protocol: "vless", streamSettings: { security: "reality" } };
    const masked = withFragment(full, true);
    expect(hasFragment(masked)).toBe(true);
    expect(masked).toMatchObject({ streamSettings: { security: "reality", finalmask: { tcp: [{ type: "fragment" }] } } });
    expect(withFragment(masked, false)).toEqual(full);
  });

  it("only offers to split a hello that exists", () => {
    expect(canFragment({ link })).toBe(true);
    expect(canFragment({ link: "warp://abc" })).toBe(false);
    expect(canFragment({ protocol: "freedom" })).toBe(false);
    expect(canFragment({ protocol: "vless", streamSettings: { security: "reality" } })).toBe(true);
    expect(canFragment({ protocol: "vless", streamSettings: { security: "none" } })).toBe(false);
    expect(canFragment({ protocol: "hysteria2", streamSettings: { security: "tls" } })).toBe(false);
  });
});

describe("rules", () => {
  it("round-trips a rule and keeps its other keys", () => {
    const stored: Json = { type: "field", domain: ["geosite:ir", "full:a.com"], port: "443", outboundTag: "direct", attrs: { x: 1 } };
    const form = fromRule(stored);
    expect(form).toMatchObject({ domain: "geosite:ir\nfull:a.com", port: "443", targetKind: "outbound", target: "direct" });
    expect(toRule(form, stored)).toEqual(stored);
  });

  it("moves a rule between an outbound and a balancer, and drops emptied conditions", () => {
    const stored: Json = { type: "field", ip: ["geoip:private"], network: "tcp", outboundTag: "block" };
    const form = { ...fromRule(stored), targetKind: "balancer" as const, target: "pool", ip: "", network: "" };
    expect(toRule(form, stored)).toEqual({ type: "field", balancerTag: "pool" });
    expect(toRule({ ...emptyRule("direct"), domain: "a.com, b.com", protocol: ["tls"], inboundTag: ["in-1"] })).toEqual({
      type: "field",
      domain: ["a.com", "b.com"],
      protocol: ["tls"],
      inboundTag: ["in-1"],
      outboundTag: "direct",
    });
  });

  it("summarises the conditions", () => {
    expect(ruleConditions({ domain: ["a", "b", "c"], port: "443", network: "udp" })).toEqual(["domain: a, b +1", "port: 443", "network: udp"]);
    expect(ruleConditions({ port: 53 })).toEqual(["port: 53"]);
    expect(ruleConditions({})).toEqual([]);
  });
});

describe("balancers and the observatory", () => {
  it("round-trip with their extras", () => {
    const stored: Json = { tag: "pool", selector: ["proxy", "warp"], strategy: { type: "leastPing", settings: { x: 1 } }, fallbackTag: "direct" };
    expect(toBalancer(fromBalancer(stored), stored)).toEqual(stored);
    expect(toBalancer({ tag: "b", selector: "a\nb", strategy: "random", fallbackTag: "" })).toEqual({
      tag: "b",
      selector: ["a", "b"],
      strategy: { type: "random" },
    });
  });

  it("keeps clean-IP settings while the form edits the rest, and switches off", () => {
    const stored: Json = { subjectSelector: ["p"], probeUrl: "https://x/204", probeInterval: "60s", cleanIp: { host: "h", candidates: ["1.1.1.1:443"] } };
    const form = fromObservatory(stored);
    expect(form).toMatchObject({ enabled: true, subjectSelector: "p", probeInterval: "60s" });
    expect(toObservatory({ ...form, probeInterval: "5m" }, stored)).toEqual({ ...stored, probeInterval: "5m" });
    expect(toObservatory({ ...form, enabled: false }, stored)).toBeUndefined();
    expect(fromObservatory(undefined).enabled).toBe(false);
  });
});

describe("dns", () => {
  it("keeps a bare address bare and writes an object once it says more", () => {
    expect(toDnsServer({ ...emptyDnsServer(), address: " 1.1.1.1 " })).toBe("1.1.1.1");
    expect(fromDnsServer("localhost").address).toBe("localhost");
    const object = toDnsServer({ ...emptyDnsServer(), address: "8.8.8.8", domains: "geosite:google\ndomain:example.com", tag: "g", skipFallback: true });
    expect(object).toEqual({ address: "8.8.8.8", domains: ["geosite:google", "domain:example.com"], tag: "g", skipFallback: true });
    const stored = { address: "9.9.9.9", expectIPs: ["geoip:ir"], domains: ["a.ir"], extra: 1 };
    const form = fromDnsServer(stored);
    expect(form.expectIPs).toBe("geoip:ir");
    expect(toDnsServer({ ...form, domains: "" }, stored)).toEqual({ address: "9.9.9.9", expectedIPs: ["geoip:ir"], extra: 1 });
  });

  it("reads and writes hosts lines", () => {
    const hosts = parseHosts("example.com 1.2.3.4\n// comment\nblocked.test #block\nmulti.test 1.1.1.1, 2.2.2.2\n");
    expect(hosts).toEqual({ "example.com": "1.2.3.4", "blocked.test": "#block", "multi.test": ["1.1.1.1", "2.2.2.2"] });
    expect(parseHosts(hostsText(hosts))).toEqual(hosts);
    expect(lines("a, b\n\n c ")).toEqual(["a", "b", "c"]);
  });
});

describe("geodata", () => {
  it("is needed by a geosite or geoip tag, but not by geoip:private", () => {
    expect(usesGeodata({ routing: { rules: [{ domain: ["geosite:ir"] }] } })).toBe(true);
    expect(usesGeodata({ routing: { rules: [{ ip: ["geoip:ir"] }] } })).toBe(true);
    expect(usesGeodata({ routing: { rules: [{ ip: ["geoip:private"] }] } })).toBe(false);
    expect(usesGeodata({ routing: { rules: [{ domain: ["domain:a.ir"] }] }, assets: { files: [{ name: "geosite.dat" }] } })).toBe(false);
    expect(usesGeodata({ dns: { servers: [{ address: "1.1.1.1", domains: ["geosite:google"] }] } })).toBe(true);
  });
});

describe("clean-IP search", () => {
  it("makes an observatory when it is turned on and takes it away when it is turned off", () => {
    const on = withCleanIp(undefined, true, ["1.1.1.1:443"], " h.example ");
    expect(on).toMatchObject({ probeUrl: "https://www.gstatic.com/generate_204", subjectSelector: [], cleanIp: { candidates: ["1.1.1.1:443"], host: "h.example", path: "/" } });
    expect(withCleanIp(on, false, [], "")).toBeUndefined();
  });

  it("leaves an observatory that measures outbounds alone", () => {
    const measuring: Json = { subjectSelector: ["proxy"], probeUrl: "https://x/204", probeInterval: "60s", cleanIp: { host: "h", candidates: [] } };
    expect(withCleanIp(measuring, false, [], "")).toEqual({ subjectSelector: ["proxy"], probeUrl: "https://x/204", probeInterval: "60s" });
    expect(withCleanIp(measuring, true, ["2.2.2.2:443"], "h2")).toMatchObject({ probeInterval: "60s", subjectSelector: ["proxy"], cleanIp: { candidates: ["2.2.2.2:443"], host: "h2" } });
  });
});

describe("dns under a proxy", () => {
  it("knows when an outbound goes through another server", () => {
    expect(hasProxyOutbound({ outbounds: [{ protocol: "freedom" }, { protocol: "blackhole" }] })).toBe(false);
    expect(hasProxyOutbound({ outbounds: [{ protocol: "freedom" }, { tag: "p", link }] })).toBe(true);
    expect(hasProxyOutbound({ outbounds: [{ protocol: "trojan", settings: {} }] })).toBe(true);
    expect(hasProxyOutbound({})).toBe(false);
    expect(isSystemResolver("localhost")).toBe(true);
    expect(isSystemResolver({ address: " LocalHost " })).toBe(true);
    expect(isSystemResolver("1.1.1.1")).toBe(false);
  });
});

describe("canonical", () => {
  it("ignores the order of keys but not of lists", () => {
    expect(canonical({ b: 1, a: { d: [1, 2], c: 3 } })).toBe(canonical({ a: { c: 3, d: [1, 2] }, b: 1 }));
    expect(canonical({ a: [1, 2] })).not.toBe(canonical({ a: [2, 1] }));
  });
});
