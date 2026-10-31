import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { FilePlus2, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import { commands, type InstalledModel } from "@/bindings";
import { cn } from "@/lib/utils";

const AUDIO_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "mp4",
  "flac",
  "ogg",
  "aac",
  "mov",
  "webm",
];

/** Pick files or drop them on the window to queue them with a chosen model. */
export function AddFiles({
  dropZone = false,
  onAdded,
  onNeedModel,
}: {
  dropZone?: boolean;
  onAdded?: () => void;
  onNeedModel?: () => void;
}) {
  const [models, setModels] = useState<InstalledModel[] | null>(null);
  const [modelId, setModelId] = useState("");
  const [hovering, setHovering] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    commands
      .modelsListInstalled()
      .then((res) => {
        if (res.status !== "ok") return;
        setModels(res.data);
        setModelId((current) => current || res.data[0]?.id || "");
      })
      .catch(() => setModels([]));
  }, []);

  const add = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0 || !modelId) return;
      setError(null);
      const res = await commands.jobsCreate({ paths, model_id: modelId });
      if (res.status === "ok") onAdded?.();
      else setError(res.error);
    },
    [modelId, onAdded],
  );

  // The listener is subscribed once; a ref lets it see the latest model.
  const addRef = useRef(add);
  addRef.current = add;

  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((e) => {
          if (e.payload.type === "enter" || e.payload.type === "over") {
            setHovering(true);
          } else if (e.payload.type === "drop") {
            setHovering(false);
            void addRef.current(e.payload.paths);
          } else {
            setHovering(false);
          }
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else stop = unlisten;
        })
        .catch(() => {});
    } catch {
      // Outside the Tauri runtime there is no window to drop files on.
    }
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  async function pick() {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Audio or video", extensions: AUDIO_EXTENSIONS }],
    });
    if (Array.isArray(picked)) await add(picked);
  }

  if (models !== null && models.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-6 text-center text-sm">
        <p className="text-muted-foreground">
          Download a model first, then add your files.
        </p>
        <Button className="mt-3" onClick={onNeedModel}>
          Choose a model
        </Button>
      </div>
    );
  }

  const controls = (
    <div className="flex flex-wrap items-center justify-center gap-2">
      <label className="sr-only" htmlFor="add-files-model">
        Model
      </label>
      <select
        id="add-files-model"
        className="h-9 rounded-md border bg-background px-2 text-sm"
        value={modelId}
        onChange={(e) => setModelId(e.target.value)}
      >
        {(models ?? []).map((m) => (
          <option key={m.id} value={m.id}>
            {m.id}
          </option>
        ))}
      </select>
      <Button onClick={pick} disabled={!modelId}>
        <FilePlus2 />
        Add files
      </Button>
    </div>
  );

  return (
    <div>
      {dropZone ? (
        <div
          className={cn(
            "rounded-lg border-2 border-dashed p-10 text-center transition-colors",
            hovering && "border-primary bg-accent",
          )}
        >
          <Upload className="mx-auto h-6 w-6 text-muted-foreground" />
          <p className="mb-4 mt-2 text-sm text-muted-foreground">
            Drop audio or video files here
          </p>
          {controls}
        </div>
      ) : (
        controls
      )}
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
