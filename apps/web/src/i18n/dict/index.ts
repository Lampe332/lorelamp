// Every dictionary, one file per area and language (`<area>.<lang>.ts`). German is the source of
// the keys; a key starts with its area's name, so two areas never share one.

import type { LangId } from "../lang";
import type { Partly } from "../types";
import { builderDe } from "./builder.de";
import { builderEn } from "./builder.en";
import { builderEs } from "./builder.es";
import { builderRu } from "./builder.ru";
import { cardsDe } from "./cards.de";
import { cardsEn } from "./cards.en";
import { cardsEs } from "./cards.es";
import { cardsRu } from "./cards.ru";
import { chatDe } from "./chat.de";
import { chatEn } from "./chat.en";
import { chatEs } from "./chat.es";
import { chatRu } from "./chat.ru";
import { commandsDe } from "./commands.de";
import { commandsEn } from "./commands.en";
import { commandsEs } from "./commands.es";
import { commandsRu } from "./commands.ru";
import { modelsDe } from "./models.de";
import { modelsEn } from "./models.en";
import { modelsEs } from "./models.es";
import { modelsRu } from "./models.ru";
import { shellDe } from "./shell.de";
import { shellEn } from "./shell.en";
import { shellEs } from "./shell.es";
import { shellRu } from "./shell.ru";
import { systemDe } from "./system.de";
import { systemEn } from "./system.en";
import { systemEs } from "./system.es";
import { systemRu } from "./system.ru";
import { tutorialDe } from "./tutorial.de";
import { tutorialEn } from "./tutorial.en";
import { tutorialEs } from "./tutorial.es";
import { tutorialRu } from "./tutorial.ru";

export const de = { ...shellDe, ...commandsDe, ...chatDe, ...cardsDe, ...modelsDe, ...systemDe, ...builderDe, ...tutorialDe };

export type Key = keyof typeof de;

export const others: Record<Exclude<LangId, "de">, Partly<typeof de>> = {
  en: { ...shellEn, ...commandsEn, ...chatEn, ...cardsEn, ...modelsEn, ...systemEn, ...builderEn, ...tutorialEn },
  es: { ...shellEs, ...commandsEs, ...chatEs, ...cardsEs, ...modelsEs, ...systemEs, ...builderEs, ...tutorialEs },
  ru: { ...shellRu, ...commandsRu, ...chatRu, ...cardsRu, ...modelsRu, ...systemRu, ...builderRu, ...tutorialRu },
};
