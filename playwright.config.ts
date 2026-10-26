import { defineConfig } from "@playwright/test";

const baseURL = process.env.ORCH_TEST_BASE_URL ?? "http://localhost:9788";

export default defineConfig({
  testDir: "./tests/api",
  timeout: 60_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    extraHTTPHeaders: process.env.ORCH_TEST_TOKEN
      ? { Authorization: `Bearer ${process.env.ORCH_TEST_TOKEN}` }
      : {},
  },
  reporter: [["list"], ["html", { open: "never" }]],
});
