import { useTranslation } from "react-i18next";
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
import { ramUsedPct, useUsage } from "@/lib/usage";
import { formatPct } from "@/lib/format";

export type ScreenId = "home" | "jobs" | "models" | "machine" | "settings";

interface NavItem {
  id: ScreenId;
  icon: LucideIcon;
}

const NAV_ITEMS: NavItem[] = [
  { id: "home", icon: Home },
  { id: "jobs", icon: ListChecks },
  { id: "models", icon: Boxes },
  { id: "machine", icon: Cpu },
  { id: "settings", icon: SettingsIcon },
];

interface SidebarProps {
  active: ScreenId;
  onNavigate: (screen: ScreenId) => void;
}

export function Sidebar({ active, onNavigate }: SidebarProps) {
  const { t } = useTranslation();
  return (
    <aside className="flex h-full w-56 shrink-0 flex-col border-r bg-card">
      <div className="flex h-14 items-center gap-2 px-4">
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <Cpu className="h-4 w-4" />
        </div>
        <span className="text-sm font-semibold">Open Audio Studio</span>
      </div>

      <nav className="flex-1 space-y-1 px-2 py-2" aria-label={t("nav.primary")}>
        {NAV_ITEMS.map(({ id, icon: Icon }) => {
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
              {t(`nav.${id}`)}
            </button>
          );
        })}
      </nav>

      <MiniMeter />

      <div className="flex items-center justify-between border-t px-3 py-3">
        <span className="text-xs text-muted-foreground">Theme</span>
        <ThemeToggle />
      </div>
    </aside>
  );
}

/** Compact live CPU/RAM bars driven by the usage feed. */
function MiniMeter() {
  const latest = useUsage((s) => s.latest);
  const cpu = latest?.cpu_total_pct ?? 0;
  const ram = ramUsedPct(latest);

  return (
    <div className="space-y-2 border-t px-3 py-3">
      <Bar label="CPU" pct={cpu} />
      <Bar label="RAM" pct={ram} />
    </div>
  );
}

function Bar({ label, pct }: { label: string; pct: number }) {
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <div>
      <div className="mb-1 flex justify-between text-[11px] text-muted-foreground">
        <span>{label}</span>
        <span>{formatPct(pct)}</span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-primary transition-all"
          style={{ width: `${clamped}%` }}
        />
      </div>
    </div>
  );
}
