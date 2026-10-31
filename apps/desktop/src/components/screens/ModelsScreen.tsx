import { useEffect, useMemo, useRef, useState } from "react";
import {
  CheckCircle2,
  Download,
  FilePlus2,
  Pause,
  Play,
  Search,
  Sparkles,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { FitBadge, QualityStars } from "@/components/FitBadge";
import { commands, type FitResult, type ModelEntry } from "@/bindings";
import { acceleratorLabel, formatBytes } from "@/lib/format";
import { speedLabel, verdictRank } from "@/lib/fit";
import { useUsage } from "@/lib/usage";
import { downloadPct, useDownloads } from "@/lib/downloads";

export function ModelsScreen() {
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [fits, setFits] = useState<Record<string, FitResult>>({});
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const [recommended, setRecommended] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [fitsOnly, setFitsOnly] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = useUsage((s) => s.latest);
  const dlById = useDownloads((s) => s.byId);
  const debounce = useRef<number | undefined>(undefined);

  async function refreshInstalled() {
    const res = await commands.modelsListInstalled();
    if (res.status === "ok") setInstalled(new Set(res.data.map((m) => m.id)));
  }

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await commands.catalogList();
        if (cancelled) return;
        setModels(list);
        setSelectedId((cur) => cur ?? list[0]?.id ?? null);
        await refreshInstalled();
        await refreshFits();
        const rec = await commands.catalogRecommend();
        if (!cancelled && rec.status === "ok") {
          setRecommended(rec.data);
          if (rec.data) setSelectedId((cur) => cur ?? rec.data);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Refresh the installed list whenever a download newly completes.
  const completedCount = Object.values(dlById).filter(
    (d) => d.state === "completed",
  ).length;
  useEffect(() => {
    void refreshInstalled();
  }, [completedCount]);

  // Recompute verdicts when live usage changes, debounced.
  useEffect(() => {
    if (!latest) return;
    window.clearTimeout(debounce.current);
    debounce.current = window.setTimeout(() => void refreshFits(), 1500);
    return () => window.clearTimeout(debounce.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latest?.captured_at_ms]);

  async function refreshFits() {
    const result = await commands.catalogFitAll();
    if (result.status === "ok") {
      const map: Record<string, FitResult> = {};
      for (const f of result.data) map[f.model_id] = f;
      setFits(map);
    }
  }

  const sorted = useMemo(() => {
    const q = query.trim().toLowerCase();
    return models
      .filter((m) => (q ? m.name.toLowerCase().includes(q) : true))
      .filter((m) => {
        if (!fitsOnly) return true;
        const v = fits[m.id]?.verdict;
        return v != null && v !== "not_recommended";
      })
      .sort((a, b) => {
        const fa = fits[a.id];
        const fb = fits[b.id];
        const ra = fa ? verdictRank(fa.verdict) : -1;
        const rb = fb ? verdictRank(fb.verdict) : -1;
        if (ra !== rb) return rb - ra;
        return b.quality_tier - a.quality_tier;
      });
  }, [models, fits, query, fitsOnly]);

  const selected = models.find((m) => m.id === selectedId) ?? null;
  const selectedFit = selectedId ? fits[selectedId] : undefined;

  return (
    <div className="mx-auto max-w-6xl px-8 py-10">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Models</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Speech models with an honest verdict for your machine, before you download.
          </p>
        </div>
      </div>

      {error && (
        <div className="mt-6 flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
          <TriangleAlert className="h-4 w-4" />
          {error}
        </div>
      )}

      <div className="mt-6 flex flex-wrap items-center gap-3">
        <div className="relative flex-1 min-w-[12rem]">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search models"
            className="h-9 w-full rounded-md border border-input bg-background pl-8 pr-3 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
        </div>
        <label className="flex items-center gap-2 text-sm text-muted-foreground">
          <input
            type="checkbox"
            checked={fitsOnly}
            onChange={(e) => setFitsOnly(e.target.checked)}
          />
          Only models that fit
        </label>
      </div>

      <div className="mt-4 grid grid-cols-1 gap-6 lg:grid-cols-[1fr_22rem]">
        <div className="space-y-2">
          {sorted.map((m) => {
            const fit = fits[m.id];
            const isRec = m.id === recommended;
            const isSel = m.id === selectedId;
            return (
              <button
                key={m.id}
                type="button"
                onClick={() => setSelectedId(m.id)}
                className={
                  "flex w-full items-center gap-4 rounded-lg border p-3 text-left transition-colors " +
                  (isSel ? "border-primary bg-accent/40" : "hover:bg-accent/30")
                }
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate font-medium">{m.name}</span>
                    {isRec && (
                      <span className="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[11px] text-primary">
                        <Sparkles className="h-3 w-3" /> Recommended
                      </span>
                    )}
                  </div>
                  <div className="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
                    <QualityStars tier={m.quality_tier} />
                    <span>{formatBytes(m.weights_bytes)}</span>
                    {fit && <span>{speedLabel(fit.rtf_est)}</span>}
                    {fit && <span>{formatBytes(fit.needed_bytes)} RAM</span>}
                  </div>
                </div>
                {installed.has(m.id) ? (
                  <span className="inline-flex items-center gap-1 text-xs text-green-600 dark:text-green-500">
                    <CheckCircle2 className="h-4 w-4" /> Installed
                  </span>
                ) : dlById[m.id]?.state === "downloading" ? (
                  <span className="text-xs text-muted-foreground">
                    {Math.round(downloadPct(dlById[m.id]) ?? 0)}%
                  </span>
                ) : (
                  fit && <FitBadge verdict={fit.verdict} />
                )}
              </button>
            );
          })}
          {sorted.length === 0 && (
            <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
              No models match.
            </div>
          )}
        </div>

        <ModelPanel
          model={selected}
          fit={selectedFit}
          installed={selected ? installed.has(selected.id) : false}
          onChanged={refreshInstalled}
        />
      </div>
    </div>
  );
}

function ModelPanel({
  model,
  fit,
  installed,
  onChanged,
}: {
  model: ModelEntry | null;
  fit: FitResult | undefined;
  installed: boolean;
  onChanged: () => void;
}) {
  const [confirmRisk, setConfirmRisk] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const dl = useDownloads((s) => (model ? s.byId[model.id] : undefined));

  useEffect(() => {
    setConfirmRisk(false);
    setConfirmDelete(false);
  }, [model?.id]);

  if (!model) {
    return (
      <Card>
        <CardContent className="p-6 text-sm text-muted-foreground">
          Select a model to see the details.
        </CardContent>
      </Card>
    );
  }

  const modelId = model.id;
  const notRecommended = fit?.verdict === "not_recommended";
  const hardBlocked = fit?.hard_blocked ?? false;
  const canDownloadDirectly = fit != null && !notRecommended;
  const activeState = dl?.state;
  const downloading = activeState === "downloading";
  const paused = activeState === "paused";

  async function startDownload() {
    setBusy(true);
    try {
      await commands.modelsDownload(modelId);
    } finally {
      setBusy(false);
    }
  }

  async function testSpeed() {
    setBusy(true);
    try {
      await commands.modelsTestSpeed(modelId);
    } finally {
      setBusy(false);
    }
  }

  async function importFromFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Model", extensions: ["bin", "gguf", "ggml"] }],
    });
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      const res = await commands.modelsImportFile(modelId, picked);
      if (res.status === "ok") onChanged();
    } finally {
      setBusy(false);
    }
  }

  async function doDelete() {
    setBusy(true);
    try {
      await commands.modelsDelete(modelId);
      onChanged();
      setConfirmDelete(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card className="h-fit lg:sticky lg:top-6">
      <CardHeader>
        <CardTitle className="flex items-center justify-between gap-2">
          <span>{model.name}</span>
          {fit && <FitBadge verdict={fit.verdict} />}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4 text-sm">
        {fit && <p>{fit.summary}</p>}

        <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
          <dt className="text-muted-foreground">Download size</dt>
          <dd className="text-right">{formatBytes(model.weights_bytes)}</dd>
          <dt className="text-muted-foreground">Needs</dt>
          <dd className="text-right">{formatBytes(fit?.needed_bytes)}</dd>
          <dt className="text-muted-foreground">Available pool</dt>
          <dd className="text-right">{formatBytes(fit?.pool_bytes)}</dd>
          <dt className="text-muted-foreground">Speed</dt>
          <dd className="text-right">
            {speedLabel(fit?.rtf_est)}{" "}
            <span className="text-xs text-muted-foreground">
              ({fit?.measured ? "Measured" : "Estimated"})
            </span>
          </dd>
          <dt className="text-muted-foreground">Runs on</dt>
          <dd className="text-right">
            {fit ? acceleratorLabel(fit.accelerator.kind) : "—"}
          </dd>
          <dt className="text-muted-foreground">License</dt>
          <dd className="text-right">{model.license}</dd>
        </dl>

        {fit && fit.reasons.length > 0 && (
          <ul className="space-y-1 text-xs text-muted-foreground">
            {fit.reasons.map((r, i) => (
              <li key={i}>• {"detail" in r ? r.detail : ""}</li>
            ))}
          </ul>
        )}

        <div className="space-y-2 pt-1">
          {/* Installed: delete flow. */}
          {installed && !confirmDelete && (
            <div className="flex items-center justify-between">
              <span className="inline-flex items-center gap-1.5 text-sm text-green-600 dark:text-green-500">
                <CheckCircle2 className="h-4 w-4" /> Installed
              </span>
              <div className="flex gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => void testSpeed()}
                >
                  Test speed
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setConfirmDelete(true)}
                >
                  <Trash2 className="h-4 w-4" /> Delete
                </Button>
              </div>
            </div>
          )}
          {installed && confirmDelete && (
            <div className="space-y-2 rounded-md border p-3 text-xs">
              <p>Delete this model's files from disk?</p>
              <div className="flex gap-2">
                <Button variant="destructive" size="sm" onClick={doDelete} disabled={busy}>
                  Delete
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setConfirmDelete(false)}>
                  Keep
                </Button>
              </div>
            </div>
          )}

          {/* In-progress download. */}
          {!installed && (downloading || paused) && (
            <div className="space-y-2">
              <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary transition-all"
                  style={{ width: `${downloadPct(dl) ?? 0}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>
                  {Math.round(downloadPct(dl) ?? 0)}%
                  {dl?.speedBps
                    ? ` · ${formatBytes(dl.speedBps)}/s`
                    : paused
                      ? " · paused"
                      : ""}
                </span>
                <div className="flex gap-1">
                  {downloading ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => commands.modelsPause(modelId)}
                    >
                      <Pause className="h-4 w-4" /> Pause
                    </Button>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => void commands.modelsResume(modelId)}
                    >
                      <Play className="h-4 w-4" /> Resume
                    </Button>
                  )}
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => commands.modelsCancel(modelId)}
                  >
                    <X className="h-4 w-4" /> Cancel
                  </Button>
                </div>
              </div>
            </div>
          )}

          {/* Idle, fits: download. */}
          {!installed && !downloading && !paused && canDownloadDirectly && (
            <Button className="w-full" onClick={startDownload} disabled={busy}>
              <Download className="h-4 w-4" />
              Download
            </Button>
          )}

          {/* Not recommended, but fits in total RAM: override. */}
          {!installed && !downloading && !paused && notRecommended && !hardBlocked && !confirmRisk && (
            <Button variant="outline" className="w-full" onClick={() => setConfirmRisk(true)}>
              Download anyway…
            </Button>
          )}
          {!installed && !downloading && !paused && notRecommended && !hardBlocked && confirmRisk && (
            <div className="space-y-2 rounded-md border border-orange-500/40 bg-orange-500/5 p-3 text-xs">
              <p className="text-orange-700 dark:text-orange-400">
                This may use swap and slow your whole computer. Continue only if you
                understand the risk.
              </p>
              <Button variant="destructive" size="sm" className="w-full" onClick={startDownload} disabled={busy}>
                I understand the risk — download
              </Button>
            </div>
          )}

          {/* Hard block: above total RAM. */}
          {!installed && notRecommended && hardBlocked && (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs text-destructive">
              This model needs more memory than your computer has. Download is
              blocked to protect your machine.
            </div>
          )}

          {dl?.state === "failed" && dl.error && (
            <p className="text-xs text-destructive">{dl.error}</p>
          )}

          {!installed && (
            <Button
              variant="ghost"
              size="sm"
              className="w-full"
              onClick={importFromFile}
              disabled={busy}
            >
              <FilePlus2 className="h-4 w-4" /> Add from file…
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
