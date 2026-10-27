import i18n from "i18next"
import LanguageDetector from "i18next-browser-languagedetector"
import { initReactI18next } from "react-i18next"
import en from "./locales/en.json"

// Ranch ships English-only at MVP. The locale picker UI lands
// in v1.x; until a second locale is added, all UI strings still flow
// through `useTranslation()` keys so retrofitting i18n later is cheap.
i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
    },
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
    },
  })

export default i18n
