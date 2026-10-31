import { create } from "zustand";

export type Theme = "light" | "dark";
export type ThemePreference = Theme | "system";

const STORAGE_KEY = "oas-theme";

function prefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: dark)").matches === true
  );
}

function resolve(preference: ThemePreference): Theme {
  if (preference !== "system") return preference;
  return prefersDark() ? "dark" : "light";
}

function readPreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark" || stored === "system") {
      return stored;
    }
  } catch {
    // localStorage may be unavailable; follow the system.
  }
  return "system";
}

function apply(preference: ThemePreference) {
  document.documentElement.classList.toggle(
    "dark",
    resolve(preference) === "dark",
  );
  try {
    localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Ignore persistence failures (e.g. private mode).
  }
}

interface ThemeState {
  preference: ThemePreference;
  theme: Theme;
  setPreference: (preference: ThemePreference) => void;
  toggle: () => void;
}

export const useTheme = create<ThemeState>((set, get) => ({
  preference: readPreference(),
  theme: resolve(readPreference()),
  setPreference: (preference) => {
    apply(preference);
    set({ preference, theme: resolve(preference) });
  },
  toggle: () =>
    get().setPreference(get().theme === "dark" ? "light" : "dark"),
}));

/** Apply the saved theme on load, and follow the system while it is chosen. */
export function initTheme() {
  const { preference, setPreference } = useTheme.getState();
  apply(preference);
  window
    .matchMedia?.("(prefers-color-scheme: dark)")
    .addEventListener?.("change", () => {
      const current = useTheme.getState().preference;
      if (current === "system") setPreference("system");
    });
}
