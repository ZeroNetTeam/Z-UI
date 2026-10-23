import { describe, expect, it } from "vitest";
import type { PanelView } from "./api";
import { addressAfterRestart } from "./panel-address";

const view: PanelView = {
  listen: "0.0.0.0:2053",
  base_path: "/zpanel",
  certificate_id: null,
  tls: false,
  locked: { listen: false, base_path: false, certificate: false },
  stored_listen: "0.0.0.0:8443",
  stored_base_path: "/secret",
  stored_certificate_id: 3,
  restart_required: true,
};

describe("the address after a restart", () => {
  it("keeps the host the browser used and takes the stored port, path and scheme", () => {
    const next = addressAfterRestart(view, new URL("http://panel.example.com:2053/zpanel/settings?x=1#top"));
    expect(next.toString()).toBe("https://panel.example.com:8443/secret/");
  });

  it("goes to plain HTTP and the root path when nothing is stored for them", () => {
    const plain = { ...view, stored_listen: "[::]:2053", stored_base_path: "/", stored_certificate_id: null };
    expect(addressAfterRestart(plain, new URL("https://[2001:db8::1]:8443/secret/")).toString()).toBe("http://[2001:db8::1]:2053/");
  });

  it("stays on HTTPS when a flag fixed the certificate, whatever is stored", () => {
    const fixed = { ...view, tls: true, stored_certificate_id: null, locked: { ...view.locked, certificate: true } };
    expect(addressAfterRestart(fixed, new URL("https://a.test/zpanel/")).protocol).toBe("https:");
  });
});
