import { mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test, type Page } from "@playwright/test";

import { installMockTauri } from "./mock-tauri";

const here = dirname(fileURLToPath(import.meta.url));
const catalog = JSON.parse(
  readFileSync(join(here, "../../../catalog/models.v1.json"), "utf8"),
);
const shots = join(here, "../../../docs/assets");
mkdirSync(shots, { recursive: true });

async function start(
  page: Page,
  options: { firstRunDone: boolean; installed: string[]; segmentCount?: number },
) {
  page.on("pageerror", (e) => {
    throw e;
  });
  await page.addInitScript(installMockTauri, {
    models: catalog.models,
    segmentCount: 3,
    ...options,
  });
  await page.goto("/");
}

test("a new user goes from first run to an exported transcript", async ({
  page,
}) => {
  await start(page, {
    firstRunDone: false,
    installed: [],
    segmentCount: 5000,
  });

  // First run: privacy promise, machine summary, one click download.
  await expect(page.getByText(/nothing is uploaded/)).toBeVisible();
  await page.screenshot({ path: join(shots, "first-run.png") });
  await page.getByRole("button", { name: "Get started" }).click();
  await expect(page.getByText(/Apple M3 Pro/)).toBeVisible();
  await page.getByRole("button", { name: "Next" }).click();
  await page.getByRole("button", { name: /^Download/ }).click();

  // Add a file with a Unicode name; the estimate shows before committing.
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Home", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Add files" }).click();
  await expect(page.getByText(/team meeting é\.wav/)).toBeVisible();
  await expect(page.getByText("Estimated time for these files")).toBeVisible();
  await page.screenshot({ path: join(shots, "add-job.png") });
  await page.getByRole("button", { name: "Add to queue" }).click();

  // The job runs and finishes on the Jobs screen.
  await expect(page.getByRole("heading", { name: "Jobs", exact: true })).toBeVisible();
  await expect(page.getByText(/Completed · whisper-tiny/)).toBeVisible();
  await page.screenshot({ path: join(shots, "jobs.png") });
  await page
    .getByRole("button", { name: /Open transcript of team meeting/ })
    .click();

  // A long transcript renders through the virtual list.
  await expect(page.getByText("Transcript line number 0", { exact: true })).toBeVisible();
  const rendered = await page.getByText(/^Transcript line number/).count();
  expect(rendered).toBeGreaterThan(5);
  expect(rendered).toBeLessThan(200);
  await page.screenshot({ path: join(shots, "transcript.png") });

  // Search reaches a line far outside the rendered window.
  await page.getByRole("searchbox").fill("number 4321");
  await expect(page.getByText("Transcript line number 4321", { exact: true })).toBeVisible();

  // Correct it, and export.
  await page.getByText("Transcript line number 4321", { exact: true }).click();
  await page.getByRole("textbox", { name: "Edit line" }).fill("Corrected line");
  await page.keyboard.press("Enter");
  await page.getByRole("searchbox").fill("corrected");
  await expect(page.getByText("Corrected line")).toBeVisible();
  await page.getByRole("button", { name: "Export SRT" }).click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __mock: { state: { exports: unknown[] } } })
            .__mock.state.exports,
      ),
    )
    .toEqual([
      { id: "job-1", format: "srt", path: "/exports/team meeting.srt" },
    ]);
});

test("the palette, settings, and dark theme work from the keyboard", async ({
  page,
}) => {
  await start(page, { firstRunDone: true, installed: ["whisper-tiny"] });
  await expect(page.getByText("Drop audio or video files here")).toBeVisible();
  await page.screenshot({ path: join(shots, "home.png") });

  await page.keyboard.press("Control+k");
  await page.getByRole("textbox", { name: "Type a command" }).fill("settings");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();

  await page.getByRole("combobox", { name: /^Theme/ }).selectOption("dark");
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.screenshot({ path: join(shots, "settings-dark.png") });
});

test("the UI stays smooth while four jobs stream text", async ({ page }) => {
  await start(page, { firstRunDone: true, installed: ["whisper-tiny"] });
  await page.getByRole("button", { name: "Jobs", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Jobs", exact: true })).toBeVisible();

  const result = await page.evaluate(async () => {
    const { emit } = (
      window as unknown as {
        __mock: { emit: (event: string, payload: unknown) => void };
      }
    ).__mock;
    const ids = ["a", "b", "c", "d"];
    ids.forEach((id, n) => {
      emit("job-state-changed", {
        job: {
          id,
          model_id: "whisper-tiny",
          input_path: `/clips/clip ${n}.wav`,
          priority: "normal",
          rank: n,
          state: "running",
          progress: 0,
          started_at_ms: 1,
        },
      });
    });
    // Each job streams at the core's limit of 4 updates a second.
    let tick = 0;
    const stream = setInterval(() => {
      tick += 1;
      for (const job_id of ids) {
        emit("job-progress-changed", {
          progress: { job_id, progress: tick / 1000, stage: "t", eta_secs: 9 },
        });
        emit("job-segments-added", {
          job_id,
          segments: [{ idx: tick, start_ms: 0, end_ms: 1, text: `line ${tick}` }],
        });
      }
    }, 250);

    let frames = 0;
    let longest = 0;
    let last = performance.now();
    const began = last;
    await new Promise<void>((done) => {
      const frame = (now: number) => {
        frames += 1;
        longest = Math.max(longest, now - last);
        last = now;
        if (now - began < 3000) requestAnimationFrame(frame);
        else done();
      };
      requestAnimationFrame(frame);
    });
    clearInterval(stream);
    return { fps: frames / 3, longest };
  });

  console.log(`four jobs: ${result.fps.toFixed(1)} fps, longest frame ${result.longest.toFixed(1)} ms`);
  expect(result.fps).toBeGreaterThan(50);
});
