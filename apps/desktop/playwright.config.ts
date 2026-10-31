import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  use: { baseURL: "http://localhost:4173", viewport: { width: 1100, height: 720 } },
  webServer: {
    // The production build, because that is what the app's CSP applies to.
    command: "pnpm exec vite build && pnpm exec vite preview --port 4173 --strictPort",
    url: "http://localhost:4173",
    reuseExistingServer: false,
  },
});
