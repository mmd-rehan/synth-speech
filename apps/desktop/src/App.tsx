import { useEffect, useState } from "react";

import { Sidebar, type ScreenId } from "@/components/Sidebar";
import { HomeScreen } from "@/components/screens/HomeScreen";
import { MachineScreen } from "@/components/screens/MachineScreen";
import { PlaceholderScreen } from "@/components/screens/PlaceholderScreen";
import { useUsage } from "@/lib/usage";

export default function App() {
  const [screen, setScreen] = useState<ScreenId>("home");
  const initUsage = useUsage((s) => s.init);

  // Start the live resource feed once, for the sidebar meter and Machine screen.
  useEffect(() => {
    initUsage();
  }, [initUsage]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      <Sidebar active={screen} onNavigate={setScreen} />
      <main className="flex-1 overflow-y-auto">
        {screen === "home" && <HomeScreen />}
        {screen === "machine" && <MachineScreen />}
        {screen !== "home" && screen !== "machine" && (
          <PlaceholderScreen screen={screen} />
        )}
      </main>
    </div>
  );
}
