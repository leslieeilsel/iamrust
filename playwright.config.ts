import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://127.0.0.1:1421",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    {
      name: "high-zoom",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 880, height: 600 },
      },
    },
  ],
  webServer: [
    {
      command: "node scripts/mock-object-store.mjs",
      url: "http://127.0.0.1:3900/health",
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
    {
      command: "cargo run -p iamrust-server --bin iamrust-server",
      url: "http://127.0.0.1:3781/health/live",
      reuseExistingServer: !process.env.CI,
      timeout: 180_000,
      env: {
        IAMRUST_BIND_ADDR: "127.0.0.1:3781",
        IAMRUST_DATA_ENCRYPTION_KEY: "iamrust-e2e-data-encryption-key-0001",
        IAMRUST_JWT_SECRET: "iamrust-e2e-jwt-signing-secret-0001",
        IAMRUST_LOG: "iamrust_server=warn",
        IAMRUST_S3_ENDPOINT: "http://127.0.0.1:3900",
      },
    },
    {
      command: "pnpm --filter @iamrust/desktop exec vite --host 127.0.0.1 --port 1421",
      url: "http://127.0.0.1:1421",
      reuseExistingServer: false,
      timeout: 120_000,
      env: {
        VITE_API_URL: "http://127.0.0.1:3781",
        VITE_DEMO_MODE: "true",
      },
    },
  ],
});
