// German left behind in a translated screen: the tests of one translation group (see docs/i18n.md).
//
// A group's spec (`i18n-<group>.spec.ts`) is a table of screens; `leftoverTests` makes one test per
// screen and language (en, es, ru). Each test puts the language in place on the server, opens the
// screen, reads every text a person sees or hears there (text, aria-label, title, placeholder, alt,
// the options of a list) and fails with each German word it finds (`german.ts`) and where it stood.
// The language goes back to German after each language's tests, so later specs find German.
//
// Runs in the graphit projects only (the words do not change with the theme): phone screens in
// `phone-graphit`, desktop and tablet screens in `desktop-graphit` (tablet: 1280 × 800 with touch).

import type { Locator, Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, test } from "./fixtures";
import { germanLeft, type Lang } from "./german";

export type Screen = {
  /** What the test is called, e.g. "Seitenleiste". */
  name: string;
  /** Where it is checked; a tablet is a 1280 × 800 window with touch. */
  form: "desktop" | "phone" | "tablet";
  /** The page to open, e.g. "/settings". */
  hash: string;
  /** Brings the screen into view once the page is open (opens a sheet, a window …) and gives back
   *  the part to read; it may match several elements. */
  region: (page: Page) => Locator | Promise<Locator>;
  /** Parts inside the region that hold the user's own content (descriptions, messages); they are
   *  not read. Names of the test data's characters and personas are left out anyway. */
  skip?: string[];
  /** Further proper names that stay as they are on this screen (a program, a site), beside `KEEP`
   *  in german.ts (shared by every group, so a group adds its names here instead). */
  keep?: string[];
};

const LANGS: Lang[] = ["en", "es", "ru"];
const TABLET = { viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true };

/** Every text a person meets in the matched elements: visible text, accessible names and hints,
 *  the options of visible lists. A part marked with another language than the page's (`lang`) is
 *  meant to stay as it is (a language's own name). */
async function textsIn(region: Locator, skip: string[]): Promise<string[]> {
  return region.evaluateAll((roots, skip) => {
    const pageLang = document.documentElement.lang;
    const out = new Set<string>();
    const left = (el: Element) => (skip.length > 0 && el.closest(skip.join(",")) !== null) || (el.closest("[lang]")?.getAttribute("lang") ?? pageLang) !== pageLang;
    const seen = (el: Element) => (el.tagName === "OPTION" ? (el.closest("select")?.checkVisibility() ?? false) : el.checkVisibility({ visibilityProperty: true }));
    for (const root of roots) {
      for (const el of [root, ...root.querySelectorAll("*")]) {
        if (left(el) || !seen(el)) continue;
        for (const attr of ["aria-label", "aria-description", "title", "placeholder", "alt"]) {
          const v = el.getAttribute(attr)?.trim();
          if (v) out.add(v);
        }
        for (const node of el.childNodes) {
          const v = node.nodeType === Node.TEXT_NODE ? node.textContent?.trim() : "";
          if (v) out.add(v);
        }
      }
    }
    return [...out];
  }, skip);
}

/** One test per screen and language for the screens of a translation group. */
export function leftoverTests(group: string, screens: Screen[]) {
  for (const lang of LANGS) {
    test.describe(`${group} (${lang})`, () => {
      test.beforeAll(() => api("PUT", "/settings", { language: lang }));
      test.afterAll(() => api("PUT", "/settings", { language: "de" }));

      for (const screen of screens) {
        test.describe(() => {
          if (screen.form === "tablet") test.use(TABLET);

          test(`${screen.name} (${screen.form}): kein Deutsch`, async ({ page, open }, info) => {
            const project = info.project.name;
            test.skip(!project.endsWith("-graphit"), "the words do not change with the theme");
            test.skip(project.startsWith("phone-") !== (screen.form === "phone"), `a ${screen.form} screen`);

            const lib = await library();
            const keep = [...lib.characters.map((c) => c.name), ...lib.personas.map((p) => p.name), ...(screen.keep ?? [])];
            await open(screen.hash);
            // The page starts in the language of its local copy, then takes the server's.
            await expect(page.locator("html")).toHaveAttribute("lang", lang);
            await expect(page.locator("html")).toHaveAttribute("data-form", screen.form);
            await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);

            const region = await screen.region(page);
            await expect(region.first()).toBeVisible();
            const hits: string[] = [];
            for (const text of await textsIn(region, screen.skip ?? [])) {
              for (const word of germanLeft(text, lang, keep)) hits.push(`"${word}" in "${text}"`);
            }
            expect(hits, `German left in ${screen.name} (${lang})`).toEqual([]);
          });
        });
      }
    });
  }
}
