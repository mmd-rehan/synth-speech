import { useCallback, useEffect, useState } from "react";

import { CommandPalette } from "@/components/CommandPalette";
import { FirstRunWizard } from "@/components/FirstRunWizard";
import { Sidebar, type ScreenId } from "@/components/Sidebar";
import { HomeScreen } from "@/components/screens/HomeScreen";
import { JobsScreen } from "@/components/screens/JobsScreen";
import { MachineScreen } from "@/components/screens/MachineScreen";
import { ModelsScreen } from "@/components/screens/ModelsScreen";
import { SettingsScreen } from "@/components/screens/SettingsScreen";
import { TranscriptScreen } from "@/components/screens/TranscriptScreen";
import { useDownloads } from "@/lib/downloads";
import { useJobs } from "@/lib/jobs";
import { useSettings } from "@/lib/settings";
import { useUsage } from "@/lib/usage";

export default function App() {
  const [screen, setScreen] = useState<ScreenId>("home");
  const [openJob, setOpenJob] = useState<string | null>(null);
  const initUsage = useUsage((s) => s.init);
  const initDownloads = useDownloads((s) => s.init);
  const initJobs = useJobs((s) => s.init);
  const settings = useSettings((s) => s.settings);
  const loadSettings = useSettings((s) => s.load);
  const updateSettings = useSettings((s) => s.update);

  // Start the live feeds once (resource usage, downloads, and jobs).
  useEffect(() => {
    initUsage();
    initDownloads();
    initJobs();
    loadSettings().catch(() => {});
  }, [initUsage, initDownloads, initJobs, loadSettings]);

  const navigate = useCallback((next: ScreenId) => {
    setOpenJob(null);
    setScreen(next);
  }, []);

  const showJob = useCallback((id: string) => {
    setScreen("jobs");
    setOpenJob(id);
  }, []);

  if (settings && !settings.first_run_done) {
    return (
      <FirstRunWizard
        onDone={(next) => {
          void updateSettings({ first_run_done: true });
          navigate(next);
        }}
      />
    );
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      <Sidebar active={screen} onNavigate={navigate} />
      <main className="flex-1 overflow-y-auto">
        {openJob ? (
          <TranscriptScreen jobId={openJob} onBack={() => setOpenJob(null)} />
        ) : (
          <>
            {screen === "home" && (
              <HomeScreen
                onJobsAdded={() => navigate("jobs")}
                onNeedModel={() => navigate("models")}
                onSeeJobs={() => navigate("jobs")}
                onOpenJob={showJob}
              />
            )}
            {screen === "jobs" && (
              <JobsScreen
                onNeedModel={() => navigate("models")}
                onOpenJob={showJob}
              />
            )}
            {screen === "machine" && <MachineScreen />}
            {screen === "models" && <ModelsScreen />}
            {screen === "settings" && <SettingsScreen />}
          </>
        )}
      </main>
      <CommandPalette onNavigate={navigate} />
    </div>
  );
}
