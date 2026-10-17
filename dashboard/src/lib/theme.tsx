import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

export type Theme = "dark" | "light" | "system";

const STORAGE_KEY = "z-ui.theme";

const ThemeContext = createContext<{ theme: Theme; setTheme: (theme: Theme) => void } | null>(
  null,
);

function stored(): Theme {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value === "light" || value === "system" ? value : "dark";
  } catch {
    return "dark";
  }
}

/** Dark by default; `system` follows the device and its changes. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(stored);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  const setTheme = (next: Theme) => {
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Private mode: the choice lasts for this page only.
    }
    setThemeState(next);
  };

  return <ThemeContext value={{ theme, setTheme }}>{children}</ThemeContext>;
}

export function useTheme() {
  const context = useContext(ThemeContext);
  if (!context) throw new Error("useTheme outside ThemeProvider");
  return context;
}
