import { useTranslation } from "react-i18next";
import { NavLink } from "react-router";
import { Logo } from "@/components/logo";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { navigation } from "./navigation";

/** The panel's navigation: the same in the desktop column and the phone sheet. */
export function SidebarContent({ onNavigate }: { onNavigate?: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-3 px-4 py-5">
        <Logo />
        <div className="min-w-0">
          <div className="font-semibold leading-tight">{t("app.name")}</div>
          <div className="truncate text-xs text-muted-foreground">{t("app.tagline")}</div>
        </div>
      </div>
      <nav className="flex-1 space-y-5 overflow-y-auto px-3 pb-4">
        {navigation.map((group) => (
          <div key={group.label}>
            <div className="px-2 pb-1.5 text-xs font-medium text-muted-foreground">
              {t(`nav.${group.label}`)}
            </div>
            <ul className="space-y-0.5">
              {group.items.map((item) => (
                <li key={item.to}>
                  {item.soon ? (
                    <span className="flex items-center gap-2.5 rounded-md px-2 py-1.5 text-sm text-muted-foreground/60">
                      <item.icon className="size-4" />
                      <span className="flex-1">{t(`nav.${item.label}`)}</span>
                      <Badge variant="outline">{t("nav.soon")}</Badge>
                    </span>
                  ) : (
                    <NavLink
                      to={item.to}
                      end={item.to === "/"}
                      onClick={onNavigate}
                      className={({ isActive }) =>
                        cn(
                          "flex items-center gap-2.5 rounded-md px-2 py-1.5 text-sm transition-colors",
                          isActive
                            ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground [&_svg]:text-primary"
                            : "text-sidebar-foreground/80 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
                        )
                      }
                    >
                      <item.icon className="size-4" />
                      {t(`nav.${item.label}`)}
                    </NavLink>
                  )}
                </li>
              ))}
            </ul>
          </div>
        ))}
      </nav>
    </div>
  );
}

export function Sidebar() {
  return (
    <aside className="sticky top-0 hidden h-svh w-64 shrink-0 border-e bg-sidebar md:block">
      <SidebarContent />
    </aside>
  );
}
