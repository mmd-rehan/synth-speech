import type { ScreenId } from "@/components/Sidebar";

const TITLES: Record<Exclude<ScreenId, "home">, string> = {
  jobs: "Jobs",
  models: "Models",
  machine: "Machine",
  settings: "Settings",
};

const DESCRIPTIONS: Record<Exclude<ScreenId, "home">, string> = {
  jobs: "Your transcription queue will appear here.",
  models: "Browse and download speech models, with a fit verdict for your machine.",
  machine: "Your hardware profile and live resource usage.",
  settings: "App preferences, storage locations, and privacy controls.",
};

export function PlaceholderScreen({
  screen,
}: {
  screen: Exclude<ScreenId, "home">;
}) {
  return (
    <div className="mx-auto max-w-3xl px-8 py-10">
      <h1 className="text-2xl font-semibold tracking-tight">{TITLES[screen]}</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        {DESCRIPTIONS[screen]}
      </p>
      <div className="mt-8 rounded-lg border border-dashed p-10 text-center text-sm text-muted-foreground">
        Coming in a later milestone.
      </div>
    </div>
  );
}
