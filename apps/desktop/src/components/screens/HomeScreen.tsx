import { useEffect, useState } from "react";
import { CheckCircle2, XCircle, Loader2 } from "lucide-react";

import { AddFiles } from "@/components/AddFiles";
import { Button } from "@/components/ui/button";
import { commands, type AppInfo } from "@/bindings";

type ProbeState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; version: string }
  | { kind: "error"; message: string };

export function HomeScreen({
  onJobsAdded,
  onNeedModel,
}: {
  onJobsAdded?: () => void;
  onNeedModel?: () => void;
}) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [probe, setProbe] = useState<ProbeState>({ kind: "idle" });

  useEffect(() => {
    let cancelled = false;
    commands
      .appVersion()
      .then((value) => {
        if (!cancelled) setInfo(value);
      })
      .catch(() => {
        // Outside the Tauri runtime (e.g. a plain browser) there is no app
        // version to read; leave it unset.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function checkWorker() {
    setProbe({ kind: "running" });
    try {
      const result = await commands.spawnWorkerProbe();
      if (result.status === "ok") {
        setProbe({ kind: "ok", version: result.data });
      } else {
        setProbe({ kind: "error", message: result.error });
      }
    } catch (err) {
      setProbe({ kind: "error", message: String(err) });
    }
  }

  return (
    <div className="mx-auto max-w-3xl px-8 py-10">
      <h1 className="text-2xl font-semibold tracking-tight">
        {info ? info.name : "Open Audio Studio"}
      </h1>
      <p className="mt-2 text-sm text-muted-foreground">
        Run speech AI models locally, on your own machine.
        {info ? ` Version ${info.version}.` : ""}
      </p>

      <section className="mt-8">
        <AddFiles dropZone onAdded={onJobsAdded} onNeedModel={onNeedModel} />
      </section>

      <section className="mt-8 rounded-lg border p-6">
        <h2 className="text-sm font-semibold">Worker sidecar</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Spawn the inference worker process and read its ready signal.
        </p>

        <div className="mt-4 flex items-center gap-3">
          <Button onClick={checkWorker} disabled={probe.kind === "running"}>
            {probe.kind === "running" && (
              <Loader2 className="h-4 w-4 animate-spin" />
            )}
            Check worker
          </Button>

          {probe.kind === "ok" && (
            <span className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-500">
              <CheckCircle2 className="h-4 w-4" />
              Worker ready (v{probe.version})
            </span>
          )}
          {probe.kind === "error" && (
            <span className="flex items-center gap-1.5 text-sm text-destructive">
              <XCircle className="h-4 w-4" />
              {probe.message}
            </span>
          )}
        </div>
      </section>
    </div>
  );
}
