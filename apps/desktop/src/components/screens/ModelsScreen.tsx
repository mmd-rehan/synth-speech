import { useEffect, useMemo, useRef, useState } from "react";
import { Download, Search, Sparkles, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { FitBadge, QualityStars } from "@/components/FitBadge";
import { commands, type FitResult, type ModelEntry } from "@/bindings";
import { acceleratorLabel, formatBytes } from "@/lib/format";
import { speedLabel, verdictRank } from "@/lib/fit";
import { useUsage } from "@/lib/usage";

export function ModelsScreen() {
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [fits, setFits] = useState<Record<string, FitResult>>({});
  const [recommended, setRecommended] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [fitsOnly, setFitsOnly] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = useUsage((s) => s.latest);
  const debounce = useRef<number | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await commands.catalogList();
        if (cancelled) return;
        setModels(list);
        setSelectedId((cur) => cur ?? list[0]?.id ?? null);
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
                {fit && <FitBadge verdict={fit.verdict} />}
              </button>
            );
          })}
          {sorted.length === 0 && (
            <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
              No models match.
            </div>
          )}
        </div>

        <ModelPanel model={selected} fit={selectedFit} />
      </div>
    </div>
  );
}

function ModelPanel({
  model,
  fit,
}: {
  model: ModelEntry | null;
  fit: FitResult | undefined;
}) {
  const [confirmRisk, setConfirmRisk] = useState(false);

  useEffect(() => {
    setConfirmRisk(false);
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

  const notRecommended = fit?.verdict === "not_recommended";
  const hardBlocked = fit?.hard_blocked ?? false;
  const canDownloadDirectly = fit != null && !notRecommended;

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
          <dd className="text-right">{speedLabel(fit?.rtf_est)}</dd>
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

        <div className="pt-1">
          {canDownloadDirectly && (
            <Button className="w-full" disabled title="Downloading arrives in M03">
              <Download className="h-4 w-4" />
              Download
            </Button>
          )}

          {notRecommended && hardBlocked && (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs text-destructive">
              This model needs more memory than your computer has. Download is
              blocked to protect your machine.
            </div>
          )}

          {notRecommended && !hardBlocked && !confirmRisk && (
            <Button
              variant="outline"
              className="w-full"
              onClick={() => setConfirmRisk(true)}
            >
              Download anyway…
            </Button>
          )}

          {notRecommended && !hardBlocked && confirmRisk && (
            <div className="space-y-2 rounded-md border border-orange-500/40 bg-orange-500/5 p-3 text-xs">
              <p className="text-orange-700 dark:text-orange-400">
                This may use swap and slow your whole computer. Continue only if
                you understand the risk.
              </p>
              <Button
                variant="destructive"
                size="sm"
                className="w-full"
                disabled
                title="Downloading arrives in M03"
              >
                I understand the risk — download
              </Button>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
