import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { commands } from "@/bindings";
import type { ScreenId } from "@/components/Sidebar";
import { useTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

interface Action {
  id: string;
  label: string;
  run: () => void;
}

/** Cmd/Ctrl+K: jump anywhere, or act on the queue, from the keyboard. */
export function CommandPalette({
  onNavigate,
}: {
  onNavigate: (screen: ScreenId) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const dialog = useRef<HTMLDialogElement>(null);
  const toggleTheme = useTheme((s) => s.toggle);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelected(0);
      dialog.current?.showModal?.();
    } else {
      dialog.current?.close?.();
    }
  }, [open]);

  const actions: Action[] = useMemo(
    () => [
      { id: "home", label: t("palette.goHome"), run: () => onNavigate("home") },
      { id: "jobs", label: t("palette.goJobs"), run: () => onNavigate("jobs") },
      {
        id: "models",
        label: t("palette.goModels"),
        run: () => onNavigate("models"),
      },
      {
        id: "machine",
        label: t("palette.goMachine"),
        run: () => onNavigate("machine"),
      },
      {
        id: "settings",
        label: t("palette.goSettings"),
        run: () => onNavigate("settings"),
      },
      {
        id: "pause",
        label: t("palette.pauseAll"),
        run: () => void commands.jobsPauseAll(),
      },
      {
        id: "resume",
        label: t("palette.resumeAll"),
        run: () => void commands.jobsResumeAll(),
      },
      { id: "theme", label: t("palette.toggleTheme"), run: toggleTheme },
    ],
    [t, onNavigate, toggleTheme],
  );

  const matches = actions.filter((a) =>
    a.label.toLowerCase().includes(query.trim().toLowerCase()),
  );

  function choose(action: Action | undefined) {
    if (!action) return;
    setOpen(false);
    action.run();
  }

  return (
    <dialog
      ref={dialog}
      aria-label={t("palette.label")}
      className="mt-[15vh] w-[28rem] max-w-[90vw] rounded-lg border bg-background p-0 text-foreground backdrop:bg-black/40"
      onCancel={() => setOpen(false)}
    >
      {open && (
        <>
          <input
            autoFocus
            aria-label={t("palette.placeholder")}
            placeholder={t("palette.placeholder")}
            className="w-full border-b bg-transparent px-4 py-3 text-sm outline-none"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelected(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((i) => Math.min(i + 1, matches.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((i) => Math.max(i - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                choose(matches[selected]);
              }
            }}
          />
          <ul className="max-h-72 overflow-y-auto p-1">
            {matches.map((action, i) => (
              <li key={action.id}>
                <button
                  className={cn(
                    "w-full rounded-md px-3 py-2 text-left text-sm",
                    i === selected && "bg-accent",
                  )}
                  onMouseEnter={() => setSelected(i)}
                  onClick={() => choose(action)}
                >
                  {action.label}
                </button>
              </li>
            ))}
            {matches.length === 0 && (
              <li className="px-3 py-2 text-sm text-muted-foreground">
                {t("palette.empty")}
              </li>
            )}
          </ul>
        </>
      )}
    </dialog>
  );
}
