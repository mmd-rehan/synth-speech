import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, ShieldCheck } from "lucide-react";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { commands, type MachineProfile, type ModelEntry } from "@/bindings";
import { formatBytes } from "@/lib/format";

/** The three first-run steps: privacy, machine scan, recommended model. */
export function FirstRunWizard({
  onDone,
}: {
  onDone: (next: "home" | "models") => void;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const [profile, setProfile] = useState<MachineProfile | null>(null);
  const [scanFailed, setScanFailed] = useState(false);
  const [model, setModel] = useState<ModelEntry | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (step !== 1) return;
    commands
      .hardwareGetProfile()
      .then((res) => {
        if (res.status === "ok") setProfile(res.data);
        else setScanFailed(true);
      })
      .catch(() => setScanFailed(true));
  }, [step]);

  useEffect(() => {
    if (step !== 2) return;
    Promise.all([commands.catalogRecommend(), commands.catalogList()])
      .then(([id, catalog]) => {
        if (id.status === "ok" && id.data) {
          setModel(catalog.find((m) => m.id === id.data) ?? null);
        }
      })
      .catch(() => {});
  }, [step]);

  async function download() {
    if (!model) return;
    setError(null);
    const res = await commands.modelsDownload(model.id);
    if (res.status === "ok") onDone("models");
    else setError(res.error);
  }

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-background p-8 text-foreground">
      <div className="w-[28rem] max-w-full rounded-lg border p-8">
        {step === 0 && (
          <>
            <ShieldCheck className="h-8 w-8 text-primary" aria-hidden />
            <h1 className="mt-3 text-xl font-semibold">
              {t("wizard.welcomeTitle")}
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">
              {t("wizard.welcomeBody")}
            </p>
            <Button className="mt-6" onClick={() => setStep(1)}>
              {t("wizard.start")}
            </Button>
          </>
        )}

        {step === 1 && (
          <>
            <h1 className="text-xl font-semibold">{t("wizard.scanTitle")}</h1>
            {profile ? (
              <p className="mt-2 text-sm">
                {t("wizard.summary", {
                  ram: formatBytes(profile.memory.total_bytes),
                  cpu: profile.cpu.brand,
                })}
              </p>
            ) : scanFailed ? (
              <p className="mt-2 text-sm">{t("wizard.scanFailed")}</p>
            ) : (
              <p className="mt-2 flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
                {t("wizard.scanning")}
              </p>
            )}
            <Button
              className="mt-6"
              disabled={!profile && !scanFailed}
              onClick={() => setStep(2)}
            >
              {t("wizard.next")}
            </Button>
          </>
        )}

        {step === 2 && (
          <>
            <h1 className="text-xl font-semibold">{t("wizard.modelTitle")}</h1>
            <p className="mt-2 text-sm text-muted-foreground">
              {model
                ? t("wizard.modelBody", { name: model.name })
                : t("wizard.noRecommendation")}
            </p>
            {error && (
              <div className="mt-3">
                <ErrorNotice message={error} />
              </div>
            )}
            <div className="mt-6 flex gap-2">
              {model && (
                <Button onClick={() => void download()}>
                  {t("wizard.download", {
                    size: formatBytes(model.weights_bytes),
                  })}
                </Button>
              )}
              <Button variant="outline" onClick={() => onDone("models")}>
                {t("wizard.chooseMyself")}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
