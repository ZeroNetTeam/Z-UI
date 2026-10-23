import type { PanelView } from "./api";

/** The address the panel will have after a restart, as this browser reaches it:
 * the same host, with the stored port, path and scheme. */
export function addressAfterRestart(view: PanelView, from: URL): URL {
  const next = new URL(from.href);
  const tls = view.locked.certificate ? view.tls : view.stored_certificate_id !== null;
  next.protocol = tls ? "https:" : "http:";
  next.port = view.stored_listen.slice(view.stored_listen.lastIndexOf(":") + 1);
  next.pathname = view.stored_base_path === "/" ? "/" : `${view.stored_base_path}/`;
  next.search = "";
  next.hash = "";
  return next;
}

/** Wait for the panel to answer at `url`, then go there. A different origin
 * cannot be read, so any answer at all (even an opaque one) is enough; and
 * after a while the browser goes anyway, to show whatever it finds. */
export function goWhenUp(url: URL) {
  let tries = 0;
  const timer = setInterval(() => {
    tries += 1;
    const go = () => {
      clearInterval(timer);
      location.assign(url.toString());
    };
    if (tries > 25) return go();
    fetch(new URL("api/auth/state", url), { mode: "no-cors", cache: "no-store" }).then(go, () => undefined);
  }, 1000);
}
