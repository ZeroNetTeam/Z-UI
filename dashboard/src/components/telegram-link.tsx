import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BotIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "@/components/copy-button";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { api, del, post, type LinkCode, type TelegramInfo } from "@/lib/api";
import { formatDate } from "@/lib/users";

const key = ["telegram"] as const;

/** Tie a Telegram chat to the signed-in admin: the chat then acts as them. */
export function TelegramLinkCard() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [code, setCode] = useState<LinkCode | null>(null);
  const info = useQuery({ queryKey: key, queryFn: () => api<TelegramInfo>("auth/telegram"), refetchInterval: code ? 3000 : false });
  const link = useMutation({ meta: { quiet: true }, mutationFn: () => post<LinkCode>("auth/telegram/link"), onSuccess: setCode });
  const unlink = useMutation({
    mutationFn: (chat: number) => del(`auth/telegram/${chat}`),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
  // Once a chat has linked, the code has done its work.
  const chats = info.data?.chats.length ?? 0;
  useEffect(() => {
    if (code && chats > 0) setCode(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chats]);

  if (info.isPending) return null;
  if (!info.data?.enabled && chats === 0) return null;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <BotIcon className="size-4 text-primary" /> {t("telegramLink.title")}
        </CardTitle>
        <CardDescription>{t("telegramLink.body")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        {info.data?.chats.map((chat) => (
          <div key={chat.chat_id} className="flex items-center justify-between gap-3 rounded-md border p-3">
            <div className="min-w-0">
              <div className="truncate text-sm font-medium" dir="auto">
                {chat.name || chat.chat_id}
              </div>
              <div className="text-xs text-muted-foreground">{t("telegramLink.since", { when: formatDate(chat.created_at, i18n.language) })}</div>
            </div>
            <Button variant="ghost" size="icon" aria-label={t("telegramLink.unlink")} disabled={unlink.isPending} onClick={() => unlink.mutate(chat.chat_id)}>
              <Trash2Icon className="text-destructive" />
            </Button>
          </div>
        ))}
        {code && (
          <div className="grid gap-2 rounded-md border border-primary/40 bg-primary/5 p-3">
            <p className="text-sm">{t("telegramLink.send")}</p>
            <code className="select-all rounded bg-background px-2 py-1 font-mono text-base" dir="ltr">
              /start {code.code}
            </code>
            <div className="flex flex-wrap gap-2">
              <CopyButton text={`/start ${code.code}`} label={t("developer.copy")} />
              {code.link && (
                <Button variant="outline" size="sm" asChild>
                  <a href={code.link} target="_blank" rel="noreferrer">
                    {t("telegramLink.open")}
                  </a>
                </Button>
              )}
            </div>
            <p className="text-xs text-muted-foreground">{t("telegramLink.expires", { minutes: Math.round(code.expires_in / 60) })}</p>
          </div>
        )}
        <FormError error={link.error ?? unlink.error} />
        {info.data?.enabled ? (
          <Button variant="outline" className="justify-self-start" disabled={link.isPending} onClick={() => link.mutate()}>
            {chats > 0 ? t("telegramLink.another") : t("telegramLink.link")}
          </Button>
        ) : (
          <p className="text-sm text-muted-foreground">{t("telegramLink.off")}</p>
        )}
      </CardContent>
    </Card>
  );
}
