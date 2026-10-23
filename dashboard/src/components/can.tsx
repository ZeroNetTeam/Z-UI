import type { ReactNode } from "react";
import { useCan } from "@/lib/permissions";

/** Show `children` only to an admin who may `perm` (`users.write`, or `sudo`). The server decides in any case. */
export function Can({ perm, children, fallback = null }: { perm: string; children: ReactNode; fallback?: ReactNode }) {
  return useCan(perm) ? children : fallback;
}
