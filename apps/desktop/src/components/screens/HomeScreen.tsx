import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { AddFiles } from "@/components/AddFiles";
import { Button } from "@/components/ui/button";
import { commands, type AppInfo } from "@/bindings";
import { fileName, useJobs } from "@/lib/jobs";

const RECENT_JOBS = 5;

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border p-4">
      <p className="text-2xl font-semibold tabular-nums">{value}</p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  );
}

export function HomeScreen({
  onJobsAdded,
  onNeedModel,
  onSeeJobs,
  onOpenJob,
}: {
  onJobsAdded?: () => void;
  onNeedModel?: () => void;
  onSeeJobs?: () => void;
  onOpenJob?: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [installed, setInstalled] = useState(0);
  const byId = useJobs((s) => s.byId);

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
    commands
      .modelsListInstalled()
      .then((res) => {
        if (!cancelled && res.status === "ok") setInstalled(res.data.length);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const jobs = useMemo(() => Object.values(byId), [byId]);
  const done = jobs.filter((j) => j.state === "completed");
  const recent = [...done]
    .sort((a, b) => (b.finished_at_ms ?? 0) - (a.finished_at_ms ?? 0))
    .slice(0, RECENT_JOBS);
  const queued = jobs.filter(
    (j) =>
      j.state === "queued" || j.state === "running" || j.state === "starting",
  ).length;

  return (
    <div className="mx-auto max-w-3xl space-y-6 px-8 py-10">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {info ? info.name : "Open Audio Studio"}
        </h1>
        <p className="mt-2 text-sm text-muted-foreground">
          {t("home.tagline")}
          {info ? ` Version ${info.version}.` : ""}
        </p>
      </div>

      <AddFiles dropZone onAdded={onJobsAdded} onNeedModel={onNeedModel} />

      <div className="grid grid-cols-3 gap-3">
        <Stat label={t("home.installed")} value={installed} />
        <Stat label={t("home.inQueue")} value={queued} />
        <Stat label={t("home.done")} value={done.length} />
      </div>

      <section>
        <div className="mb-2 flex items-center justify-between">
          <h2 className="text-sm font-semibold">{t("home.recent")}</h2>
          {jobs.length > 0 && (
            <Button size="sm" variant="ghost" onClick={onSeeJobs}>
              {t("home.seeAll")}
            </Button>
          )}
        </div>
        {recent.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("home.noJobs")}</p>
        ) : (
          <ul className="divide-y rounded-lg border">
            {recent.map((job) => (
              <li key={job.id}>
                <button
                  className="w-full truncate px-4 py-2 text-left text-sm hover:bg-accent"
                  onClick={() => onOpenJob?.(job.id)}
                >
                  {fileName(job.input_path)}
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
