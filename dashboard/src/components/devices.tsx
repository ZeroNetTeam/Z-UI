import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { SmartphoneIcon, Trash2Icon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { api, del, type Device, type User } from "@/lib/api";
import { relative } from "@/lib/users";

/** The devices a user's subscription was fetched on, to forget. */
export function Devices({ user }: { user: User }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const key = ["users", user.id, "devices"];
  const devices = useQuery({ queryKey: key, queryFn: () => api<Device[]>(`users/${user.id}/devices`) });
  const forget = useMutation({
    mutationFn: (id: number | null) => del(id === null ? `users/${user.id}/devices` : `users/${user.id}/devices/${id}`),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
  const list = devices.data ?? [];
  const number = (value: number) => new Intl.NumberFormat(i18n.language).format(value);
  return (
    <div className="grid min-w-0 gap-2">
      <div className="flex items-center justify-between gap-2 text-sm">
        <span className="font-medium">
          {t("devices.title")}{" "}
          <span className="text-muted-foreground">
            ({number(list.length)}
            {user.hwid_limit > 0 ? ` / ${number(user.hwid_limit)}` : ""})
          </span>
        </span>
        {list.length > 1 && (
          <Button type="button" variant="ghost" size="sm" onClick={() => forget.mutate(null)} disabled={forget.isPending}>
            {t("devices.forgetAll")}
          </Button>
        )}
      </div>
      <FormError error={forget.error ?? devices.error} />
      {list.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("devices.none")}</p>
      ) : (
        <ul className="grid min-w-0 gap-1">
          {list.map((device) => (
            <li key={device.id} className="flex min-w-0 items-center gap-3 rounded-md border px-3 py-2">
              <SmartphoneIcon className="size-4 shrink-0 text-primary/80" />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm" dir="auto">
                  {[device.model, device.platform, device.os_version].filter(Boolean).join(" · ") || device.hwid}
                </div>
                <div className="truncate text-xs text-muted-foreground" dir="auto">
                  {t("devices.lastSeen", { when: relative(device.last_seen, i18n.language) })}
                  {device.user_agent ? ` · ${device.user_agent}` : ""}
                </div>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="size-8"
                aria-label={t("devices.forget")}
                onClick={() => forget.mutate(device.id)}
                disabled={forget.isPending}
              >
                <Trash2Icon className="text-destructive" />
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
