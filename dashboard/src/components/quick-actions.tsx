import {
  BookmarkIcon,
  BoxesIcon,
  FileKey2Icon,
  KeyRoundIcon,
  LayoutTemplateIcon,
  NetworkIcon,
  RouteIcon,
  ShieldCheckIcon,
  UserPlusIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { can, useMe } from "@/lib/permissions";
import { cn } from "@/lib/utils";

interface Action {
  id: string;
  icon: LucideIcon;
  /** Where it goes: the page, with `?new=…` to open its create dialog. */
  to: string;
  /** The permission it takes (`users.write`, or `sudo`); absent for any admin. */
  needs?: string;
}

const groups: { id: string; actions: Action[] }[] = [
  {
    id: "users",
    actions: [
      { id: "user", icon: UserPlusIcon, to: "/users?new=user", needs: "users.write" },
      { id: "group", icon: BoxesIcon, to: "/groups?new=group", needs: "groups.write" },
      { id: "template", icon: LayoutTemplateIcon, to: "/templates?new=template", needs: "templates.write" },
    ],
  },
  {
    id: "server",
    actions: [
      { id: "inbound", icon: WaypointsIcon, to: "/inbounds?new=inbound", needs: "inbounds.write" },
      { id: "host", icon: NetworkIcon, to: "/hosts?new=host", needs: "hosts.write" },
      { id: "outbound", icon: RouteIcon, to: "/core?new=outbound", needs: "core.write" },
      { id: "certificate", icon: FileKey2Icon, to: "/certificates?new=certificate", needs: "certificates.write" },
    ],
  },
  {
    id: "management",
    actions: [
      { id: "admin", icon: ShieldCheckIcon, to: "/admins?new=admin", needs: "sudo" },
      { id: "apiKey", icon: KeyRoundIcon, to: "/developer?tab=keys", needs: "api.write" },
    ],
  },
];

/** The dashboard's corner button: shortcuts to everything one makes. */
export function QuickActions() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const admin = useMe();
  // Only what this admin may make; a group with nothing left is not shown.
  const shown = groups
    .map((group) => ({ ...group, actions: group.actions.filter((action) => !action.needs || can(admin, action.needs)) }))
    .filter((group) => group.actions.length > 0);
  if (shown.length === 0) return null;
  return (
    <>
      <Button variant="outline" onClick={() => setOpen(true)}>
        <BookmarkIcon /> {t("quick.title")}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title={t("quick.title")} description={t("quick.subtitle")} className="max-w-2xl">
          <div className="grid gap-5">
            {shown.map((group) => (
              <section key={group.id} aria-labelledby={`quick-${group.id}`}>
                <h3 id={`quick-${group.id}`} className="mb-2 text-xs font-medium text-muted-foreground">
                  {t(`quick.group_${group.id}`)}
                </h3>
                <div className="grid gap-2 sm:grid-cols-2">
                  {group.actions.map((action) => (
                    <button
                      key={action.id}
                      type="button"
                      onClick={() => {
                        setOpen(false);
                        void navigate(action.to);
                      }}
                      className={cn(
                        "flex items-center gap-3 rounded-lg border p-3 text-start transition-colors hover:border-primary/50 hover:bg-accent",
                      )}
                    >
                      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                        <action.icon className="size-4" />
                      </span>
                      <span className="min-w-0">
                        <span className="flex items-center gap-2 text-sm font-medium">
                          {t(`quick.${action.id}`)}
                        </span>
                        <span className="block text-xs text-muted-foreground">{t(`quick.${action.id}Hint`)}</span>
                      </span>
                    </button>
                  ))}
                </div>
              </section>
            ))}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
