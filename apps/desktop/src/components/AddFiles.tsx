import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { FilePlus2, Upload } from "lucide-react";

import { AddJobDialog } from "@/components/AddJobDialog";
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

/** Pick files or drop them on the window, then confirm in the Add dialog. */
export function AddFiles({
  dropZone = false,
  onAdded,
  onNeedModel,
}: {
  dropZone?: boolean;
  onAdded?: () => void;
  onNeedModel?: () => void;
}) {
  const { t } = useTranslation();
  const [models, setModels] = useState<InstalledModel[] | null>(null);
  const [hovering, setHovering] = useState(false);
  const [paths, setPaths] = useState<string[] | null>(null);
  const hasModels = (models?.length ?? 0) > 0;
  const canDrop = useRef(false);
  canDrop.current = hasModels;

  useEffect(() => {
    commands
      .modelsListInstalled()
      .then((res) => setModels(res.status === "ok" ? res.data : []))
      .catch(() => setModels([]));
  }, []);

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
            if (canDrop.current && e.payload.paths.length > 0) {
              setPaths(e.payload.paths);
            }
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

  const pick = useCallback(async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Audio or video", extensions: AUDIO_EXTENSIONS }],
    });
    if (Array.isArray(picked) && picked.length > 0) setPaths(picked);
  }, []);

  if (models !== null && !hasModels) {
    return (
      <div className="rounded-lg border border-dashed p-6 text-center text-sm">
        <p className="text-muted-foreground">{t("add.needModel")}</p>
        <Button className="mt-3" onClick={onNeedModel}>
          {t("add.chooseModel")}
        </Button>
      </div>
    );
  }

  const button = (
    <Button onClick={() => void pick()} disabled={!hasModels}>
      <FilePlus2 />
      {t("add.addFiles")}
    </Button>
  );

  return (
    <>
      {dropZone ? (
        <div
          className={cn(
            "rounded-lg border-2 border-dashed p-10 text-center transition-colors",
            hovering && "border-primary bg-accent",
          )}
        >
          <Upload className="mx-auto h-6 w-6 text-muted-foreground" />
          <p className="mb-4 mt-2 text-sm text-muted-foreground">
            {t("add.drop")}
          </p>
          {button}
        </div>
      ) : (
        button
      )}
      {paths && models && (
        <AddJobDialog
          paths={paths}
          installed={models}
          onClose={() => setPaths(null)}
          onAdded={onAdded}
        />
      )}
    </>
  );
}
