import { useEffect, useState } from "react";
import {
  Cpu,
  MemoryStick,
  HardDrive,
  MonitorCog,
  Thermometer,
  RefreshCw,
  ClipboardCopy,
  Zap,
  TriangleAlert,
} from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { commands, type MachineProfile } from "@/bindings";
import { acceleratorLabel, formatBytes, formatPct } from "@/lib/format";
import { ramUsedPct, useUsage } from "@/lib/usage";

function SpecCard({
  icon: Icon,
  title,
  children,
}: {
  icon: typeof Cpu;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-sm text-muted-foreground">
          <Icon className="h-4 w-4" />
          {title}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-sm">{children}</CardContent>
    </Card>
  );
}

export function MachineScreen() {
  const [profile, setProfile] = useState<MachineProfile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const history = useUsage((s) => s.history);

  useEffect(() => {
    void load(false);
  }, []);

  async function load(refresh: boolean) {
    setBusy(true);
    setError(null);
    try {
      const result = refresh
        ? await commands.hardwareRefresh()
        : await commands.hardwareGetProfile();
      if (result.status === "ok") setProfile(result.data);
      else setError(result.error);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function copyDiagnostics() {
    try {
      const result = await commands.copyDiagnostics();
      if (result.status === "ok") {
        await navigator.clipboard.writeText(result.data);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }
    } catch {
      // ignore; clipboard may be unavailable
    }
  }

  const chartData = history.map((s, i) => ({
    i,
    cpu: Math.round(s.cpu_total_pct ?? 0),
    ram: Math.round(ramUsedPct(s)),
  }));

  return (
    <div className="mx-auto max-w-5xl px-8 py-10">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Machine</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Your hardware and how it is being used right now.
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={copyDiagnostics}>
            <ClipboardCopy className="h-4 w-4" />
            {copied ? "Copied" : "Copy diagnostics"}
          </Button>
          <Button size="sm" onClick={() => load(true)} disabled={busy}>
            <RefreshCw className={busy ? "h-4 w-4 animate-spin" : "h-4 w-4"} />
            Rescan
          </Button>
        </div>
      </div>

      {error && (
        <div className="mt-6 flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
          <TriangleAlert className="h-4 w-4" />
          {error}
        </div>
      )}

      {profile && (
        <>
          <div className="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <SpecCard icon={MonitorCog} title="Operating system">
              <div className="font-medium">{profile.os.name}</div>
              <div className="text-muted-foreground">
                {profile.os.version} · {profile.os.arch}
              </div>
            </SpecCard>

            <SpecCard icon={Cpu} title="Processor">
              <div className="font-medium">{profile.cpu.brand}</div>
              <div className="text-muted-foreground">
                {profile.cpu.physical_cores} cores / {profile.cpu.logical_cores}{" "}
                threads
              </div>
            </SpecCard>

            <SpecCard icon={MemoryStick} title="Memory">
              <div className="font-medium">
                {formatBytes(profile.memory.total_bytes)}
                {profile.memory.is_unified ? " unified" : ""}
              </div>
              <div className="text-muted-foreground">
                {formatBytes(profile.memory.available_bytes)} available
              </div>
            </SpecCard>

            <SpecCard icon={HardDrive} title="Disk">
              <div className="font-medium">
                {formatBytes(profile.disk.free_bytes)} free
              </div>
              <div className="text-muted-foreground">on the models volume</div>
            </SpecCard>

            <SpecCard icon={Thermometer} title="Thermal &amp; power">
              <div className="font-medium capitalize">{profile.thermal}</div>
              <div className="text-muted-foreground">
                {profile.power.on_battery ? "On battery" : "Plugged in"}
                {profile.power.battery_pct != null
                  ? ` · ${profile.power.battery_pct}%`
                  : ""}
              </div>
            </SpecCard>

            <SpecCard icon={Zap} title="Accelerators">
              {profile.accel.length > 0 ? (
                <ul className="space-y-0.5">
                  {profile.accel.map((a) => (
                    <li key={a.kind} className="font-medium">
                      {acceleratorLabel(a.kind)}
                    </li>
                  ))}
                </ul>
              ) : (
                <div className="text-muted-foreground">CPU only</div>
              )}
            </SpecCard>
          </div>

          {profile.gpus.some((g) => !g.usable) && (
            <div className="mt-4 space-y-1">
              {profile.gpus
                .filter((g) => !g.usable)
                .map((g, i) => (
                  <div
                    key={i}
                    className="flex items-center gap-2 text-sm text-muted-foreground"
                  >
                    <TriangleAlert className="h-4 w-4" />
                    GPU found but not usable: {g.name}
                    {g.not_usable_reason ? ` — ${g.not_usable_reason}` : ""}
                  </div>
                ))}
            </div>
          )}

          <Card className="mt-6">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Live usage</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="h-56 w-full">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={chartData}>
                    <defs>
                      <linearGradient id="cpuFill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="5%" stopColor="hsl(var(--primary))" stopOpacity={0.4} />
                        <stop offset="95%" stopColor="hsl(var(--primary))" stopOpacity={0} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
                    <XAxis dataKey="i" hide />
                    <YAxis domain={[0, 100]} width={32} tickFormatter={(v) => `${v}`} />
                    <Tooltip
                      formatter={(value: number, name: string) => [
                        `${value}%`,
                        name === "cpu" ? "CPU" : "RAM",
                      ]}
                      labelFormatter={() => ""}
                    />
                    <Area
                      type="monotone"
                      dataKey="cpu"
                      stroke="hsl(var(--primary))"
                      fill="url(#cpuFill)"
                      isAnimationActive={false}
                    />
                    <Area
                      type="monotone"
                      dataKey="ram"
                      stroke="hsl(var(--muted-foreground))"
                      fillOpacity={0}
                      isAnimationActive={false}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
              <div className="mt-2 flex gap-6 text-xs text-muted-foreground">
                <span>
                  CPU {formatPct(history.at(-1)?.cpu_total_pct)} · RAM{" "}
                  {formatPct(ramUsedPct(history.at(-1) ?? null))}
                </span>
              </div>
            </CardContent>
          </Card>
        </>
      )}

      {!profile && !error && (
        <div className="mt-10 text-sm text-muted-foreground">Scanning hardware…</div>
      )}
    </div>
  );
}
