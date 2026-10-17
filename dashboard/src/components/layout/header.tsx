import { LanguagesIcon, MenuIcon, MonitorIcon, MoonIcon, SunIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { languages, setLanguage, type Language } from "@/i18n";
import { useTheme, type Theme } from "@/lib/theme";
import { SidebarContent } from "./sidebar";

export function Header() {
  const { t, i18n } = useTranslation();
  const { theme, setTheme } = useTheme();
  const [menuOpen, setMenuOpen] = useState(false);
  const ThemeIcon = theme === "light" ? SunIcon : theme === "dark" ? MoonIcon : MonitorIcon;

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/80 px-4 backdrop-blur md:px-6">
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
    </header>
  );
}
