import { useState } from "react";

import { Sidebar, type ScreenId } from "@/components/Sidebar";
import { HomeScreen } from "@/components/screens/HomeScreen";
import { PlaceholderScreen } from "@/components/screens/PlaceholderScreen";

export default function App() {
  const [screen, setScreen] = useState<ScreenId>("home");

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      <Sidebar active={screen} onNavigate={setScreen} />
      <main className="flex-1 overflow-y-auto">
        {screen === "home" ? (
          <HomeScreen />
        ) : (
          <PlaceholderScreen screen={screen} />
        )}
      </main>
    </div>
  );
}
