import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Copy } from "lucide-react";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { commands, type JobSegment } from "@/bindings";
import { fileName, filterSegments, formatTimestamp, useJobs } from "@/lib/jobs";
import { cn } from "@/lib/utils";

const EXPORT_FORMATS = ["txt", "srt", "vtt", "json"] as const;

function Line({
  segment,
  active,
  onSeek,
  onSave,
}: {
  segment: JobSegment;
  active: boolean;
  onSeek: () => void;
  onSave: (text: string) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<string | null>(null);
  // Removing a focused input fires blur, so Enter or Escape must settle the edit
  // once and leave that late blur with nothing to do.
  const settled = useRef(false);
  const time = formatTimestamp(segment.start_ms ?? 0);

  function startEdit() {
    settled.current = false;
    setDraft(segment.text);
  }

  function finish(save: boolean) {
    if (settled.current) return;
    settled.current = true;
    if (save && draft !== null && draft !== segment.text) onSave(draft);
    setDraft(null);
  }

  return (
    <div className={cn("flex gap-3 rounded px-2 py-1", active && "bg-accent")}>
      <button
        className="w-16 shrink-0 text-left font-mono text-xs leading-6 text-muted-foreground hover:text-foreground"
        aria-label={t("transcript.seek", { time })}
        onClick={onSeek}
      >
        {time}
      </button>
      {draft === null ? (
        <button
          className="min-w-0 flex-1 cursor-text text-left text-sm leading-6"
          aria-label={`${t("transcript.edit")}: ${segment.text}`}
          onClick={startEdit}
        >
          {segment.text}
        </button>
      ) : (
        <input
          autoFocus
          aria-label={t("transcript.edit")}
          className="min-w-0 flex-1 rounded border bg-background px-1 text-sm leading-6"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => finish(true)}
          onKeyDown={(e) => {
            if (e.key === "Enter") finish(true);
            if (e.key === "Escape") finish(false);
          }}
        />
      )}
    </div>
  );
}

/** Read, search, correct, hear, and export one transcript. */
export function TranscriptScreen({
  jobId,
  onBack,
}: {
  jobId: string;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const job = useJobs((s) => s.byId[jobId]);
  const segments = useJobs((s) => s.segments[jobId]);
  const loadSegments = useJobs((s) => s.loadSegments);
  const setSegmentText = useJobs((s) => s.setSegmentText);
  const [query, setQuery] = useState("");
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [audioFailed, setAudioFailed] = useState(false);
  const [nowMs, setNowMs] = useState(0);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void loadSegments(jobId);
    commands
      .jobsAllowPlayback(jobId)
      .then((res) => {
        if (res.status === "ok") setAudioUrl(convertFileSrc(res.data));
        else setAudioFailed(true);
      })
      .catch(() => setAudioFailed(true));
  }, [jobId, loadSegments]);

  const visible = useMemo(
    () => filterSegments(segments ?? [], query),
    [segments, query],
  );

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 32,
    initialRect: { width: 800, height: 600 },
    overscan: 10,
  });

  function seek(segment: JobSegment) {
    if (!audio.current) return;
    audio.current.currentTime = (segment.start_ms ?? 0) / 1000;
    void audio.current.play().catch(() => {});
  }

  async function saveLine(segment: JobSegment, text: string) {
    const res = await commands.jobsUpdateSegment(jobId, segment.idx, text);
    if (res.status === "ok") setSegmentText(jobId, segment.idx, text);
    else setError(res.error);
  }

  async function copyAll() {
    const text = (segments ?? []).map((s) => s.text).join("\n");
    await navigator.clipboard.writeText(text);
    setCopied(true);
  }

  async function exportAs(format: (typeof EXPORT_FORMATS)[number]) {
    const stem = job ? fileName(job.input_path).replace(/\.[^.]+$/, "") : "transcript";
    const path = await save({
      defaultPath: `${stem}.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (!path) return;
    const res = await commands.jobsExport(jobId, format, path);
    setError(res.status === "ok" ? null : res.error);
  }

  return (
    <div className="mx-auto flex h-full max-w-4xl flex-col gap-3 px-8 py-6">
      <div>
        <Button size="sm" variant="ghost" onClick={onBack}>
          <ArrowLeft />
          {t("transcript.back")}
        </Button>
        <h1 className="mt-1 truncate text-xl font-semibold">
          {job ? fileName(job.input_path) : ""}
        </h1>
      </div>

      {audioUrl && !audioFailed && (
        <audio
          ref={audio}
          controls
          src={audioUrl}
          className="w-full"
          onTimeUpdate={(e) => setNowMs(e.currentTarget.currentTime * 1000)}
          onError={() => setAudioFailed(true)}
        />
      )}
      {audioFailed && (
        <p className="text-sm text-muted-foreground">{t("transcript.noAudio")}</p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <input
          type="search"
          aria-label={t("transcript.search")}
          placeholder={t("transcript.search")}
          className="h-9 min-w-48 flex-1 rounded-md border bg-background px-3 text-sm"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Button size="sm" variant="outline" onClick={() => void copyAll()}>
          <Copy />
          {copied ? t("transcript.copied") : t("transcript.copy")}
        </Button>
        {EXPORT_FORMATS.map((format) => (
          <Button
            key={format}
            size="sm"
            variant="outline"
            aria-label={`${t("transcript.export")} ${format.toUpperCase()}`}
            onClick={() => void exportAs(format)}
          >
            {format.toUpperCase()}
          </Button>
        ))}
      </div>

      {error && <ErrorNotice message={error} />}

      {segments && segments.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("transcript.empty")}</p>
      )}
      {segments && segments.length > 0 && visible.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t("transcript.noMatches")}
        </p>
      )}

      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto">
        <div
          className="relative w-full"
          style={{ height: virtualizer.getTotalSize() }}
        >
          {virtualizer.getVirtualItems().map((row) => {
            const segment = visible[row.index];
            const active =
              nowMs > 0 &&
              nowMs >= (segment.start_ms ?? 0) &&
              nowMs < (segment.end_ms ?? 0);
            return (
              <div
                key={segment.idx}
                ref={virtualizer.measureElement}
                data-index={row.index}
                className="absolute left-0 top-0 w-full"
                style={{ transform: `translateY(${row.start}px)` }}
              >
                <Line
                  segment={segment}
                  active={active}
                  onSeek={() => seek(segment)}
                  onSave={(text) => void saveLine(segment, text)}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
