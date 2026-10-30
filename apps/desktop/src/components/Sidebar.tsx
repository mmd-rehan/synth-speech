import {
  Home,
  ListChecks,
  Boxes,
  Cpu,
  Settings as SettingsIcon,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { ThemeToggle } from "@/components/ThemeToggle";

export type ScreenId = "home" | "jobs" | "models" | "machine" | "settings";

interface NavItem {
  id: ScreenId;
  label: string;
  icon: LucideIcon;
}

const NAV_ITEMS: NavItem[] = [
  { id: "home", label: "Home", icon: Home },
  { id: "jobs", label: "Jobs", icon: ListChecks },
  { id: "models", label: "Models", icon: Boxes },
  { id: "machine", label: "Machine", icon: Cpu },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

interface SidebarProps {
  active: ScreenId;
  onNavigate: (screen: ScreenId) => void;
}

export function Sidebar({ active, onNavigate }: SidebarProps) {
  return (
    <aside className="flex h-full w-56 shrink-0 flex-col border-r bg-card">
      <div className="flex h-14 items-center gap-2 px-4">
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <Cpu className="h-4 w-4" />
        </div>
        <span className="text-sm font-semibold">Open Audio Studio</span>
      </div>

      <nav className="flex-1 space-y-1 px-2 py-2" aria-label="Primary">
        {NAV_ITEMS.map(({ id, label, icon: Icon }) => {
          const isActive = id === active;
          return (
            <button
              key={id}
              type="button"
              onClick={() => onNavigate(id)}
              aria-current={isActive ? "page" : undefined}
              className={cn(
                "flex w-full items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground",
              )}
            >
              <Icon className="h-4 w-4" />
              {label}
            </button>
          );
        })}
      </nav>

      <div className="flex items-center justify-between border-t px-3 py-3">
        <span className="text-xs text-muted-foreground">Theme</span>
        <ThemeToggle />
      </div>
    </aside>
  );
}
