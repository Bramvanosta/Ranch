import { useTheme } from "next-themes"

import { useSetTheme } from "@/lib/settingsSync"
import { cn } from "@/lib/utils"

const THEMES = [
  { value: "system", label: "System", hint: "Follows your OS" },
  { value: "light", label: "Light", hint: "" },
  { value: "dark", label: "Dark", hint: "" },
] as const

/**
 * Settings › Appearance: light, dark, or follow the OS.
 * Applied at once and remembered (`next-themes`, mounted at the app root).
 */
export function AppearanceSection() {
  const { theme } = useTheme()
  const setTheme = useSetTheme()
  const current = theme ?? "system"
  return (
    <section className="flex flex-col gap-4" data-testid="settings-appearance">
      <header>
        <h2 className="text-lg font-semibold">Appearance</h2>
        <p className="text-sm text-(--text2)">Theme of the whole app.</p>
      </header>
      <fieldset className="flex gap-2">
        <legend className="sr-only">Theme</legend>
        {THEMES.map((t) => (
          <label
            key={t.value}
            className={cn(
              "flex cursor-pointer flex-col gap-0.5 rounded-md border border-(--line) px-4 py-2 text-sm",
              "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-(--accent)",
              current === t.value && "border-(--accent) bg-(--accent)/5",
            )}
          >
            <span className="flex items-center gap-2">
              <input
                type="radio"
                name="theme"
                value={t.value}
                checked={current === t.value}
                onChange={() => setTheme(t.value)}
                data-testid={`theme-${t.value}`}
              />
              <span className="font-medium">{t.label}</span>
            </span>
            {t.hint ? <span className="pl-6 text-xs text-(--text2)">{t.hint}</span> : null}
          </label>
        ))}
      </fieldset>
    </section>
  )
}
