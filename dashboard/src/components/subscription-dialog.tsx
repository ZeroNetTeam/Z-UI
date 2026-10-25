import { useQuery } from "@tanstack/react-query";
import { CheckIcon, CopyIcon, QrCodeIcon, TriangleAlertIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FormError } from "@/components/form-error";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type User, type UserSubscription } from "@/lib/api";

export function useUserSubscription(user: User | null) {
  return useQuery({
    queryKey: ["users", user?.id, "subscription", user?.sub_token],
    queryFn: () => api<UserSubscription>(`users/${user!.id}/subscription`),
    enabled: user !== null,
  });
}

export function CopyIconButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className="size-8 shrink-0"
      aria-label={label}
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? <CheckIcon className="text-success" /> : <CopyIcon />}
    </Button>
  );
}

/** An SVG the panel drew (a QR code), on white so phones can read it. */
function Svg({ svg, size = 200 }: { svg: string; size?: number }) {
  return (
    <div
      className="rounded-lg bg-white p-2 [&_svg]:block [&_svg]:h-auto [&_svg]:w-full"
      style={{ width: size }}
      // The panel makes this SVG from a QR code; it carries no user markup.
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

function LinkQr({ link }: { link: string }) {
  const qr = useQuery({
    queryKey: ["qr", link],
    queryFn: () => api<{ svg: string }>(`tools/qr?text=${encodeURIComponent(link)}`),
    staleTime: Infinity,
  });
  return qr.data ? <Svg svg={qr.data.svg} size={180} /> : <Skeleton className="size-[180px]" />;
}

/** A user's subscription link and every config in it, to copy or scan. */
export function SubscriptionDialog({ user, onOpenChange }: { user: User | null; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const subscription = useUserSubscription(user);
  const [shown, setShown] = useState<string | null>(null);
  const data = subscription.data;
  return (
    <Dialog open={user !== null} onOpenChange={onOpenChange}>
      <DialogContent title={t("subscription.title", { name: user?.username })} className="max-w-xl">
        <FormError error={subscription.error} />
        {!data ? (
          <Skeleton className="h-64" />
        ) : (
          <div className="grid gap-5">
            <div className="grid justify-items-center gap-3">
              <Svg svg={data.qr_svg} size={220} />
              <div className="flex w-full items-center gap-1 rounded-md border bg-muted/40 px-3 py-1.5">
                <code dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs">
                  {data.url}
                </code>
                <CopyIconButton text={data.url} label={t("subscription.copyUrl")} />
              </div>
              <p className="text-center text-xs text-muted-foreground">{t("subscription.urlHint")}</p>
              {data.local && (
                <p role="alert" className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 p-3 text-xs">
                  <TriangleAlertIcon className="mt-0.5 size-4 shrink-0 text-warning" />
                  <span>{t("subscription.localWarning")}</span>
                </p>
              )}
            </div>
            <div className="grid gap-2">
              <div className="text-sm font-medium">{t("subscription.configs", { count: data.links.length })}</div>
              {data.links.length === 0 ? (
                <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">{t("subscription.noConfigs")}</p>
              ) : (
                <ul className="grid gap-1">
                  {data.links.map((item) => (
                    <li key={item.link} className="rounded-md border">
                      <div className="flex items-center gap-1 px-3 py-1">
                        <span className="min-w-0 flex-1 truncate text-sm" dir="auto">
                          {item.remark}
                        </span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="size-8"
                          aria-label={t("subscription.qr")}
                          aria-pressed={shown === item.link}
                          onClick={() => setShown(shown === item.link ? null : item.link)}
                        >
                          <QrCodeIcon />
                        </Button>
                        <CopyIconButton text={item.link} label={t("subscription.copyLink")} />
                      </div>
                      {shown === item.link && (
                        <div className="grid justify-items-center border-t p-3">
                          <LinkQr link={item.link} />
                        </div>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
