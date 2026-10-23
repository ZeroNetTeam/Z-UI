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
