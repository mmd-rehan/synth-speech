import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { ErrorNotice } from "@/components/ErrorNotice";
import { FitBadge } from "@/components/FitBadge";
import { Button } from "@/components/ui/button";
import {
  commands,
  type FitResult,
  type InstalledModel,
  type ModelEntry,
} from "@/bindings";
import { estimateSecs, fileName, formatDuration } from "@/lib/jobs";
import { cn } from "@/lib/utils";

const LANGUAGES: [string, string][] = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["ar", "Arabic"],
  ["ur", "Urdu"],
  ["hi", "Hindi"],
  ["zh", "Chinese"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["ru", "Russian"],
];

interface Choice {
  id: string;
  name: string;
  fit: FitResult | null;
}

export function AddJobDialog({
  paths,
  installed,
  onClose,
  onAdded,
}: {
  paths: string[];
  installed: InstalledModel[];
  onClose: () => void;
  onAdded?: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const [catalog, setCatalog] = useState<ModelEntry[]>([]);
  const [fits, setFits] = useState<FitResult[]>([]);
  const [modelId, setModelId] = useState(installed[0]?.id ?? "");
  const [translate, setTranslate] = useState(false);
  const [language, setLanguage] = useState("");
  const [durations, setDurations] = useState<(number | null)[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    dialog.current?.showModal?.();
  }, []);

  useEffect(() => {
    commands.catalogList().then(setCatalog).catch(() => {});
    commands
      .catalogFitAll()
      .then((res) => {
        if (res.status === "ok") setFits(res.data);
      })
      .catch(() => {});
    commands
      .jobsProbeDurations(paths)
      .then(setDurations)
      .catch(() => {});
  }, [paths]);

  const choices: Choice[] = useMemo(
    () =>
      installed.map((m) => ({
        id: m.id,
        name: catalog.find((c) => c.id === m.id)?.name ?? m.id,
        fit: fits.find((f) => f.model_id === m.id) ?? null,
      })),
    [installed, catalog, fits],
  );

  const secs = estimateSecs(
    durations,
    choices.find((c) => c.id === modelId)?.fit?.rtf_est,
  );

  async function submit() {
    setBusy(true);
    setError(null);
    const res = await commands.jobsCreate({
      paths,
      model_id: modelId,
      translate,
      language: language || null,
    });
    setBusy(false);
    if (res.status === "ok") {
      onAdded?.();
      onClose();
    } else {
      setError(res.error);
    }
  }

  return (
    <dialog
      ref={dialog}
      aria-label={t("add.title", { count: paths.length })}
      className="w-[32rem] max-w-[90vw] rounded-lg border bg-background p-6 text-foreground backdrop:bg-black/40"
      onCancel={onClose}
    >
      <h2 className="text-lg font-semibold">
        {t("add.title", { count: paths.length })}
      </h2>
      <ul className="mt-2 max-h-24 overflow-y-auto text-sm text-muted-foreground">
        {paths.map((p) => (
          <li key={p} className="truncate">
            {fileName(p)}
          </li>
        ))}
      </ul>

      <fieldset className="mt-4">
        <legend className="mb-1 text-sm font-medium">{t("add.model")}</legend>
        <div className="max-h-44 space-y-1 overflow-y-auto">
          {choices.map((choice) => (
            <label
              key={choice.id}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-sm",
                choice.id === modelId && "border-primary bg-accent",
              )}
            >
              <input
                type="radio"
                name="model"
                checked={choice.id === modelId}
                onChange={() => setModelId(choice.id)}
              />
              <span className="flex-1">{choice.name}</span>
              {choice.fit && <FitBadge verdict={choice.fit.verdict} />}
            </label>
          ))}
        </div>
      </fieldset>

      <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
        <label>
          <span className="mb-1 block font-medium">{t("add.task")}</span>
          <select
            className="h-9 w-full rounded-md border bg-background px-2"
            value={translate ? "translate" : "transcribe"}
            onChange={(e) => setTranslate(e.target.value === "translate")}
          >
            <option value="transcribe">{t("add.transcribe")}</option>
            <option value="translate">{t("add.translate")}</option>
          </select>
        </label>
        <label>
          <span className="mb-1 block font-medium">{t("add.language")}</span>
          <select
            className="h-9 w-full rounded-md border bg-background px-2"
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
          >
            <option value="">{t("add.auto")}</option>
            {LANGUAGES.map(([code, name]) => (
              <option key={code} value={code}>
                {name}
              </option>
            ))}
          </select>
        </label>
      </div>

      <p className="mt-4 text-sm text-muted-foreground">
        {secs != null
          ? t("add.estimate", { time: formatDuration(secs) })
          : t("add.estimateUnknown")}
      </p>

      {error && (
        <div className="mt-3">
          <ErrorNotice message={error} />
        </div>
      )}

      <div className="mt-5 flex justify-end gap-2">
        <Button variant="outline" onClick={onClose}>
          {t("add.cancel")}
        </Button>
        <Button onClick={() => void submit()} disabled={busy || !modelId}>
          {t("add.submit")}
        </Button>
      </div>
    </dialog>
  );
}
