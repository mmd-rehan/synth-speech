import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, Loader2 } from "lucide-react";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { commands, type Settings } from "@/bindings";
import { useSettings } from "@/lib/settings";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-lg border p-5">
      <h2 className="mb-3 text-sm font-semibold">{title}</h2>
      <div className="space-y-3 text-sm">{children}</div>
    </section>
  );
}

function NumberField({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-4">
      <span>{label}</span>
      <input
        type="number"
        min={min}
        max={max}
        className="h-8 w-20 rounded-md border bg-background px-2"
        value={value}
        onChange={(e) => {
          const next = Number(e.target.value);
          if (Number.isInteger(next) && next >= min && next <= max) {
            onChange(next);
          }
        }}
      />
    </label>
  );
}

export function SettingsScreen() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const load = useSettings((s) => s.load);
  const update = useSettings((s) => s.update);
  const [error, setError] = useState<string | null>(null);
  const [moved, setMoved] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const [probe, setProbe] = useState<
    | { kind: "idle" | "running" }
    | { kind: "ok"; version: string }
    | { kind: "error"; message: string }
  >({ kind: "idle" });

  useEffect(() => {
    commands
      .appVersion()
      .then((info) => setVersion(info.version))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!settings) void load().catch(() => setError(t("error.unknown")));
  }, [settings, load, t]);

  async function change(patch: Partial<Settings>) {
    setError(await update(patch));
  }

  async function moveModels() {
    const dir = await open({ directory: true });
    if (typeof dir !== "string") return;
    const res = await commands.settingsMoveModels(dir);
    if (res.status === "ok") {
      setMoved(true);
      await load();
    } else {
      setError(res.error);
    }
  }

  async function checkWorker() {
    setProbe({ kind: "running" });
    const res = await commands.spawnWorkerProbe();
    setProbe(
      res.status === "ok"
        ? { kind: "ok", version: res.data }
        : { kind: "error", message: res.error },
    );
  }

  if (!settings) {
    return (
      <div className="mx-auto max-w-3xl px-8 py-10">
        {error ? (
          <ErrorNotice
            message={error}
            actionLabel={t("error.retry")}
            onAction={() => void load().then(() => setError(null))}
          />
        ) : (
          <Loader2 className="h-5 w-5 animate-spin" aria-label="Loading" />
        )}
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-4 px-8 py-10">
      <h1 className="text-2xl font-semibold tracking-tight">
        {t("settings.title")}
      </h1>
      {error && <ErrorNotice message={error} />}

      <Section title={t("settings.general")}>
        <label className="flex items-center justify-between gap-4">
          <span>{t("settings.theme")}</span>
          <select
            className="h-8 rounded-md border bg-background px-2"
            value={settings.theme}
            onChange={(e) => void change({ theme: e.target.value })}
          >
            <option value="system">{t("settings.themeSystem")}</option>
            <option value="light">{t("settings.themeLight")}</option>
            <option value="dark">{t("settings.themeDark")}</option>
          </select>
        </label>
      </Section>

      <Section title={t("settings.models")}>
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <p>{t("settings.modelsFolder")}</p>
            <p className="truncate text-xs text-muted-foreground">
              {settings.models_dir ?? t("settings.modelsFolderDefault")}
            </p>
          </div>
          <Button size="sm" variant="outline" onClick={() => void moveModels()}>
            {t("settings.changeFolder")}
          </Button>
        </div>
        {moved && <p role="status">{t("settings.restart")}</p>}
      </Section>

      <Section title={t("settings.performance")}>
        <label className="flex items-center justify-between gap-4">
          <span>{t("settings.gpuPreference")}</span>
          <select
            className="h-8 rounded-md border bg-background px-2"
            value={settings.gpu_preference}
            onChange={(e) => void change({ gpu_preference: e.target.value })}
          >
            <option value="auto">{t("settings.gpuAuto")}</option>
            <option value="cpu">{t("settings.gpuCpu")}</option>
          </select>
        </label>
        <NumberField
          label={t("settings.threadCap")}
          min={1}
          max={64}
          value={settings.thread_cap_per_job}
          onChange={(v) => void change({ thread_cap_per_job: v })}
        />
        <NumberField
          label={t("settings.batteryFloor")}
          min={0}
          max={100}
          value={settings.pause_on_battery_below}
          onChange={(v) => void change({ pause_on_battery_below: v })}
        />
      </Section>

      <Section title={t("settings.network")}>
        <label className="flex items-center justify-between gap-4">
          <span>{t("settings.offline")}</span>
          <input
            type="checkbox"
            checked={settings.offline_mode}
            onChange={(e) => void change({ offline_mode: e.target.checked })}
          />
        </label>
      </Section>

      <Section title={t("settings.privacy")}>
        <p className="text-muted-foreground">{t("settings.privacyBody")}</p>
      </Section>

      <Section title={t("settings.advanced")}>
        <NumberField
          label={t("settings.retention")}
          min={0}
          max={3650}
          value={settings.retention_days}
          onChange={(v) => void change({ retention_days: v })}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button
            size="sm"
            variant="outline"
            disabled={probe.kind === "running"}
            onClick={() => void checkWorker()}
          >
            {t("settings.checkWorker")}
          </Button>
          {probe.kind === "ok" && (
            <span className="flex items-center gap-1.5 text-green-700 dark:text-green-500">
              <CheckCircle2 className="h-4 w-4" aria-hidden />
              {t("settings.workerReady", { version: probe.version })}
            </span>
          )}
        </div>
        {probe.kind === "error" && <ErrorNotice message={probe.message} />}
      </Section>

      <Section title={t("settings.about")}>
        {version && <p>{t("settings.version", { version })}</p>}
        <p className="text-muted-foreground">{t("settings.license")}</p>
        <p className="text-muted-foreground">{t("settings.source")}</p>
        <p className="text-muted-foreground">{t("settings.crashHelp")}</p>
      </Section>
    </div>
  );
}
