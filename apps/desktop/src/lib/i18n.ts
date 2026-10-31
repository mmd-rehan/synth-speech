import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "@/locales/en.json";

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en } },
  lng: "en",
  fallbackLng: "en",
  // React already escapes what it renders.
  interpolation: { escapeValue: false },
});

export default i18n;
