import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import fa from "./locales/fa.json";

export const languages = {
  fa: { label: "فارسی", dir: "rtl" },
  en: { label: "English", dir: "ltr" },
} as const;

export type Language = keyof typeof languages;

const STORAGE_KEY = "z-ui.language";

function stored(): Language | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value && value in languages ? (value as Language) : null;
  } catch {
    return null;
  }
}

/** Point the page at `language`: its `lang`, and `dir` for right-to-left. */
function applyToDocument(language: Language) {
  document.documentElement.lang = language;
  document.documentElement.dir = languages[language].dir;
}

export function setLanguage(language: Language) {
  try {
    localStorage.setItem(STORAGE_KEY, language);
  } catch {
    // Private mode: the choice lasts for this page only.
  }
  void i18n.changeLanguage(language);
}

const initial = stored() ?? "fa";
void i18n.use(initReactI18next).init({
  resources: { fa: { translation: fa }, en: { translation: en } },
  lng: initial,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});
applyToDocument(initial);
i18n.on("languageChanged", (language) => applyToDocument(language as Language));

export default i18n;
