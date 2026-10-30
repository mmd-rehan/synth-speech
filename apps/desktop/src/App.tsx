import { useEffect, useState } from "react";

import { Sidebar, type ScreenId } from "@/components/Sidebar";
import { HomeScreen } from "@/components/screens/HomeScreen";
import { MachineScreen } from "@/components/screens/MachineScreen";
import { ModelsScreen } from "@/components/screens/ModelsScreen";
import { PlaceholderScreen } from "@/components/screens/PlaceholderScreen";
import { useUsage } from "@/lib/usage";
import { useDownloads } from "@/lib/downloads";

export default function App() {
  const [screen, setScreen] = useState<ScreenId>("home");
  const initUsage = useUsage((s) => s.init);
  const initDownloads = useDownloads((s) => s.init);

  // Start the live feeds once (resource usage + download events).
  useEffect(() => {
    initUsage();
    initDownloads();
  }, [initUsage, initDownloads]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      <Sidebar active={screen} onNavigate={setScreen} />
      <main className="flex-1 overflow-y-auto">
        {screen === "home" && <HomeScreen />}
        {screen === "machine" && <MachineScreen />}
        {screen === "models" && <ModelsScreen />}
        {screen !== "home" && screen !== "machine" && screen !== "models" && (
          <PlaceholderScreen screen={screen} />
        )}
      </main>
    </div>
  );
}
