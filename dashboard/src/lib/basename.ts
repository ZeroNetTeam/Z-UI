/** The path the panel is served under, from the <base> the server set. */
export function basename(): string {
  const path = new URL(document.baseURI).pathname.replace(/\/$/, "");
  return path || "/";
}
