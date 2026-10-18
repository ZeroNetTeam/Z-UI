import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type AuthState } from "./api";

export const authKey = ["auth", "state"] as const;

/** Whether the panel needs setting up, and who is signed in. */
export function useAuth() {
  return useQuery({ queryKey: authKey, queryFn: () => api<AuthState>("auth/state"), staleTime: 60_000 });
}

/** After signing in or out: everything cached belongs to the old session. */
export function useAuthChanged() {
  const client = useQueryClient();
  return () => {
    client.removeQueries({ predicate: (query) => query.queryKey[0] !== "auth" });
    return client.invalidateQueries({ queryKey: authKey });
  };
}
