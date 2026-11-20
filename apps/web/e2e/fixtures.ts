// Shared test fixture: every test runs in its project's theme and opens pages with the token.

import { test as base, expect, type Locator, type Page } from "@playwright/test";
import { setTheme, token } from "./api";

export type Options = { theme: string };

export const test = base.extend<Options & { open: (hash: string) => Promise<void> }>({
  theme: ["graphit", { option: true }],
  open: async ({ page, theme }, use) => {
    await setTheme(theme);
    await use(async (hash: string) => {
      await page.goto(`/?token=${token}#${hash.replace(/^#/, "")}`);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    });
  },
});

export { expect };

export const isPhone = (page: Page) => (page.viewportSize()?.width ?? 0) < 768;

/** The model-state pill: in the sidebar on a desktop, in the top bar on a phone. */
export const modelStatus = (page: Page) =>
  isPhone(page) ? page.locator("header.top").getByRole("status") : page.getByRole("complementary").getByRole("status");

/**
 * Screenshot comparison of the visible screen. Everything that changes from run to run is masked:
 * clock times, counters and the model's measurements (they depend on the CPU and earlier runs),
 * including the benchmark's table and recommendations.
 */
export async function snap(page: Page, name: string, extra: Locator[] = []) {
  await page.evaluate(() => document.fonts.ready);
  await expect(page).toHaveScreenshot(`${name}.png`, {
    mask: [
      page.getByText(/\d{1,2}:\d{2}/),
      page.locator("dd, .tile-big, .cache-meter, .toast"),
      // The benchmark's measurements (and whether one of them is set) differ from run to run.
      page.locator(".bm-note, .bm-picks, .bm-table-wrap"),
      page.getByText(/Tokens\/s/),
      ...extra,
    ],
  });
}
