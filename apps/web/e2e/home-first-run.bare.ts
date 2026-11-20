// The very first start, in all five layouts: nothing imported and no model set up. Runs only through
// e2e/first-run.config.ts against a server with an empty data folder (see there).

import { mkdirSync } from "node:fs";
import { api, library, setLayout } from "./api";
import { expect, test } from "./fixtures";

const LAYOUTS = ["bisher", "dashboard", "bento", "buehne", "kommando"] as const;
const SHOTS = process.env.RP_E2E_SHOTS;

/** The welcome tour's step (Q9): an empty server opens it over the start screen, so it waits meanwhile. */
let tour = "";

test.beforeAll(async () => {
  const lib = await library();
  const engine = await api<{ managed: boolean; profile: unknown; status: string }>("GET", "/engine");
  // Never run against a server that has data: this test is about the empty one.
  if (lib.characters.length > 0 || engine.profile || !engine.managed || engine.status === "ready") {
    throw new Error("first-run test needs an empty data folder and a server started with --manage-engine --llama http://127.0.0.1:1");
  }
  tour = (await api<{ step: string }>("GET", "/tutorial")).step;
  await api("PUT", "/tutorial", { step: "done" });
});

test.afterAll(async () => {
  await setLayout("bisher");
  if (tour) await api("PUT", "/tutorial", { step: tour });
});

for (const layout of LAYOUTS) {
  test(`Erster Start, ${layout}: Modell einrichten, dann Figur importieren`, async ({ page, open }) => {
    await setLayout(layout);
    await open("/");
    // The four steps of the welcome tour, none done, the model first. It is said once, in plain words:
    // without the program that runs models, "Modell einrichten" is the one button and fetches it first.
    const first = page.getByRole("region", { name: "Hier beginnt deine erste Geschichte" });
    await expect(first).toContainText("0 von 4 erledigt");
    await expect(first.getByRole("listitem")).toHaveCount(4);
    const model = first.getByRole("listitem").first();
    await expect(model).toContainText("Noch kein Modell eingerichtet");
    await expect(model).toContainText("Dafür lädt Lorelamp zuerst das Programm");
    await expect(model).not.toContainText("Engine");
    await expect(model.getByRole("button", { name: "Modell einrichten" })).toBeVisible();
    await expect(page.locator("a, button").filter({ visible: true, hasText: /^\s*Modell einrichten\s*$/ })).toHaveCount(1);
    // "User" is the name a new installation starts with: the name step is open, with its way on.
    const name = first.getByRole("listitem").nth(2);
    await expect(name).toContainText("Deinen Namen festlegen");
    await expect(name.getByRole("link", { name: "Namen festlegen" })).toHaveAttribute("href", /^#\/personas\/\d+$/);
    await expect(first.getByRole("button", { name: "Karte importieren" })).toBeVisible();
    await expect(page.getByRole("button", { name: /^Start-Layout wechseln/ })).toBeVisible();
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: `${SHOTS}/erster-start-${layout}-${test.info().project.name}.png`, animations: "disabled" });
    }
    await page.getByRole("region", { name: "Hier beginnt deine erste Geschichte" }).getByRole("button", { name: "Karte importieren" }).click();
    await expect(page).toHaveURL(/#\/import$/);
  });
}
