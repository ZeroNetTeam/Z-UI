import {
  LanguagesIcon,
  LogOutIcon,
  MenuIcon,
  MonitorIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
  UserIcon,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { languages, setLanguage, type Language } from "@/i18n";
import { post } from "@/lib/api";
import { useAuth, useAuthChanged } from "@/lib/auth";
import { useTheme, type Theme } from "@/lib/theme";
import { SidebarContent } from "./sidebar";

/** The language and theme menus, also on the sign-in pages. */
export function HeaderMenus() {
  const { t, i18n } = useTranslation();
  const { theme, setTheme } = useTheme();
  const ThemeIcon = theme === "light" ? SunIcon : theme === "dark" ? MoonIcon : MonitorIcon;
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" aria-label={t("header.language")}>
            <LanguagesIcon />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuLabel>{t("header.language")}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={i18n.language}
            onValueChange={(value) => setLanguage(value as Language)}
          >
            {Object.entries(languages).map(([code, { label }]) => (
              <DropdownMenuRadioItem key={code} value={code}>
                {label}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" aria-label={t("header.theme")}>
            <ThemeIcon />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuLabel>{t("header.theme")}</DropdownMenuLabel>
          <DropdownMenuRadioGroup value={theme} onValueChange={(value) => setTheme(value as Theme)}>
            <DropdownMenuRadioItem value="dark">
              <MoonIcon /> {t("header.dark")}
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="light">
              <SunIcon /> {t("header.light")}
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="system">
              <MonitorIcon /> {t("header.system")}
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}

function UserMenu() {
  const { t } = useTranslation();
  const admin = useAuth().data?.admin;
  const authChanged = useAuthChanged();
  if (!admin) return null;
  const signOut = async () => {
    await post("auth/logout").catch(() => undefined);
    await authChanged();
  };
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" className="gap-2 px-2" aria-label={t("auth.account")}>
          <span className="flex size-7 items-center justify-center rounded-full bg-primary/15 text-primary">
            <UserIcon className="size-4" />
          </span>
          <span className="hidden text-sm sm:inline" dir="ltr">
            {admin.username}
          </span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-44">
        <DropdownMenuLabel className="text-start">
          <span dir="ltr" className="block">
            {admin.username}
          </span>
          <span className="block text-xs font-normal text-muted-foreground">
            {admin.is_sudo ? t("access.kindSudo") : (admin.role?.name ?? "")}
          </span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/settings">
            <SettingsIcon /> {t("nav.settings")}
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void signOut()} className="text-destructive">
          <LogOutIcon /> {t("auth.signOut")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function Header() {
  const { t } = useTranslation();
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-1 border-b bg-background/80 px-4 backdrop-blur md:px-6">
      <Sheet open={menuOpen} onOpenChange={setMenuOpen}>
        <SheetTrigger asChild>
          <Button variant="ghost" size="icon" className="md:hidden" aria-label={t("header.menu")}>
            <MenuIcon />
          </Button>
        </SheetTrigger>
        <SheetContent>
          <SheetTitle className="sr-only">{t("header.menu")}</SheetTitle>
          <SidebarContent onNavigate={() => setMenuOpen(false)} />
        </SheetContent>
      </Sheet>

      <div className="flex-1" />
      <HeaderMenus />
      <UserMenu />
    </header>
  );
}
