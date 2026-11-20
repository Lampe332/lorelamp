// Character settings › "Du in dieser Geschichte" with a card that names the user often: the result
// in one sentence, one row per topic, the card's sentences folded away, its writing rules apart,
// and the choice of what applies saved at once. The SFW test card (e2e/userfit-card.json) is
// imported here and deleted again, so the library the other tests see stays as it was.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { api, library } from "./api";
import { expect, test } from "./fixtures";

const NAME = "Henrike Thal";
const card = () => new Uint8Array(readFileSync(fileURLToPath(new URL("./userfit-card.json", import.meta.url))));

const ids = async () => (await library()).characters.filter((c) => c.name === NAME).map((c) => c.id);

test.afterEach(async () => {
  for (const id of await ids()) await api("DELETE", `/characters/${id}`);
});

test("Du in dieser Geschichte: Themen, Belege, Regeln und Vorrang", async ({ page, open }) => {
  await api("POST", "/characters/import", card());
  const [id] = await ids();
  expect(id).toBeDefined();
  await open(`/char/${id}/settings`);

  const fit = page.locator('[data-spot="user-fit"]');
  const head = fit.getByRole("status").first();
  // The card calls the user "Jonas", the active persona is called otherwise: a contradiction.
  await expect(head).toContainText("Widerspruch");
  await expect(head).toContainText("Die Karte macht dich zu: „Jonas“, neuer Kollege, Lehrling, männlich.");

  // One row per topic, in this order; the role comes only from the card.
  const topics = fit.getByRole("list", { name: "Was die Karte über dich sagt" });
  await expect(topics.getByRole("heading", { level: 3 })).toHaveText(["Rolle & Beziehung", "Name", "Geschlecht", "Aussehen", "Persönlichkeit"]);
  const role = topics.getByRole("listitem").filter({ has: page.getByRole("heading", { name: "Rolle & Beziehung" }) });
  await expect(role.getByRole("list", { name: "Rolle & Beziehung laut Karte" }).getByRole("listitem")).toHaveText(["neuer Kollege", "Lehrling"]);
  await expect(role).toContainText("aus der Karte");
  const name = topics.getByRole("listitem").filter({ has: page.getByRole("heading", { name: "Name" }) });
  await expect(name).toContainText("widerspricht");

  // The sentences are folded; opening shows each once, the writing rules are not among them.
  const proofs = role.locator("details.uf-proof");
  await expect(proofs.locator("summary")).toHaveText("Belege (5)");
  // "{{user}} ist freundlich" counts as a role only for its word "freund"; it shows under Persönlichkeit.
  await expect(proofs.getByText("ist freundlich, aber manchmal stur", { exact: false })).toHaveCount(0);
  await expect(proofs.getByText("ist ihr Lehrling in der Werkstatt", { exact: false })).toBeHidden();
  await proofs.locator("summary").click();
  await expect(proofs.getByText("ist ihr Lehrling in der Werkstatt", { exact: false })).toBeVisible();
  await expect(role.getByText("Schreibe niemals", { exact: false })).toHaveCount(0);

  // The card's rules about the user are a group of their own, folded too; keyboard opens it.
  const rules = fit.locator("details.uf-rules");
  await expect(rules.locator("summary")).toContainText("Regeln der Karte zu dir");
  await expect(rules.locator("summary small")).toHaveText("3");
  await expect(rules.getByText("Schreibe niemals Handlungen", { exact: false })).toBeHidden();
  await rules.locator("summary").focus();
  await page.keyboard.press("Enter");
  await expect(rules.getByText("Schreibe niemals Handlungen", { exact: false })).toBeVisible();

  // What applies in the chat: a segmented choice, saved at once, with one line about the choice.
  const rule = page.getByRole("radiogroup", { name: "Was im Chat gilt" });
  await expect(rule.getByRole("radio")).toHaveCount(3);
  await expect(rule.getByRole("radio", { name: "Wie in der Persona" })).toBeChecked();
  await expect(fit.getByText(/^Standard: folgt der Persona/)).toBeVisible();
  const cardWins = rule.getByText("Die Karte gilt", { exact: true });
  await cardWins.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await cardWins.click();
  await expect(rule.getByRole("radio", { name: "Die Karte gilt" })).toBeChecked();
  await expect(fit.getByText(/^Wie die Karte dich beschreibt, zählt/)).toBeVisible();
  await expect(fit.getByRole("status").filter({ hasText: /^Gespeichert\./ })).toBeVisible();
  await expect.poll(async () => (await api<{ info: { user_priority: string | null } }>("GET", `/characters/${id}`)).info.user_priority).toBe("card");
  // The arrow keys move the choice like any radio group, and that saves too.
  await page.keyboard.press("ArrowLeft");
  await expect(rule.getByRole("radio", { name: "Meine Persona gilt" })).toBeChecked();
  await expect.poll(async () => (await api<{ info: { user_priority: string | null } }>("GET", `/characters/${id}`)).info.user_priority).toBe("persona");
});
