import { describe, expect, it } from "vitest";
import type { Inbound } from "./api";
import { defaults, fromInbound, summary, toPayload, visionPossible } from "./inbound-form";

function stored(payload: ReturnType<typeof toPayload>): Inbound {
  return { ...payload, id: 1, tag: "t", created_at: 0, updated_at: 0 };
}

describe("inbound form", () => {
  it("builds a VLESS REALITY inbound with Vision", () => {
    const form = {
      ...defaults("vless"),
      remark: " Main ",
      port: 443,
      realityPrivateKey: "priv",
      realityPublicKey: "pub",
      realityShortIds: "ab12, cd34",
    };
    const payload = toPayload(form);
    expect(payload.remark).toBe("Main");
    expect(payload.settings).toEqual({ decryption: "none", flow: "xtls-rprx-vision" });
    expect(payload.stream).toMatchObject({
      network: "raw",
      security: "reality",
      realitySettings: { target: "www.microsoft.com:443", serverNames: ["www.microsoft.com"], shortIds: ["ab12", "cd34"] },
    });
  });

  it("drops Vision where it cannot work", () => {
    const form = { ...defaults("vless"), transport: "ws" as const };
    expect(visionPossible(form)).toBe(false);
    expect(toPayload(form).settings).toEqual({ decryption: "none" });
  });

  it("carries a TLS certificate by id and splits ALPN", () => {
    const payload = toPayload({ ...defaults("trojan"), transport: "ws", path: "/ray", host: "cdn.example", certificateId: 4 });
    expect(payload.stream).toEqual({
      network: "ws",
      wsSettings: { path: "/ray", headers: { Host: "cdn.example" } },
      security: "tls",
      tlsSettings: { certificateId: 4, alpn: ["h2", "http/1.1"] },
    });
  });

  it("keeps a server key only for Shadowsocks 2022", () => {
    expect(toPayload({ ...defaults("shadowsocks"), ssPassword: "k" }).settings).toEqual({
      method: "2022-blake3-aes-128-gcm",
      password: "k",
    });
    expect(toPayload({ ...defaults("shadowsocks"), ssMethod: "aes-256-gcm", ssPassword: "k" }).settings).toEqual({
      method: "aes-256-gcm",
    });
  });

  it("gives Hysteria2 TLS and no transport", () => {
    const payload = toPayload({ ...defaults("hysteria2"), certificateId: 2, obfsPassword: "salt" });
    expect(payload.stream).toEqual({ security: "tls", tlsSettings: { certificateId: 2, alpn: ["h2", "http/1.1"] } });
    expect(payload.settings).toEqual({ obfs: { type: "salamander", password: "salt" } });
  });

  it("reads back what it wrote", () => {
    for (const form of [
      { ...defaults("vless"), remark: "a", port: 443, realityPrivateKey: "p", realityShortIds: "ab" },
      { ...defaults("vmess"), remark: "b", transport: "grpc" as const, serviceName: "svc", security: "tls" as const, certificateId: 1 },
      { ...defaults("shadowsocks"), remark: "c", ssPassword: "key" },
      { ...defaults("wireguard"), remark: "d", wgSecretKey: "s", wgPublicKey: "p", wgMtu: 1380 },
    ]) {
      const again = fromInbound(stored(toPayload(form)));
      expect(toPayload(again)).toEqual(toPayload(form));
    }
  });

  it("summarises transport and security", () => {
    expect(summary(stored(toPayload({ ...defaults("vless") })))).toBe("raw · reality");
    expect(summary(stored(toPayload({ ...defaults("shadowsocks") })))).toBe("—");
  });
});
