/** What is left of the week the old panel's files are kept: whole days, and the hours after them. */
export function windowLeft(expiresAt: number | null, now = Date.now() / 1000): { days: number; hours: number; over: boolean } {
  if (expiresAt == null) return { days: 0, hours: 0, over: false };
  const left = Math.max(0, expiresAt - now);
  return { days: Math.floor(left / 86_400), hours: Math.floor((left % 86_400) / 3600), over: left === 0 };
}
