import { create } from "zustand";

import { commands, type Settings } from "@/bindings";
import { useTheme, type ThemePreference } from "@/lib/theme";

interface SettingsStore {
  settings: Settings | null;
  load: () => Promise<void>;
  /** Change settings and save them. Returns an error message on failure. */
  update: (patch: Partial<Settings>) => Promise<string | null>;
}

function syncTheme(theme: string) {
  if (theme === "light" || theme === "dark" || theme === "system") {
    useTheme.getState().setPreference(theme as ThemePreference);
  }
}

export const useSettings = create<SettingsStore>((set, get) => ({
  settings: null,

  load: async () => {
    const settings = await commands.settingsGet();
    syncTheme(settings.theme);
    set({ settings });
  },

  update: async (patch) => {
    const current = get().settings;
    if (!current) return "Settings are not loaded yet.";
    const next = { ...current, ...patch };
    set({ settings: next });
    if (patch.theme) syncTheme(patch.theme);
    const res = await commands.settingsSet(next);
    if (res.status === "error") {
      set({ settings: current });
      return res.error;
    }
    return null;
  },
}));
