import type { Fully } from "../types";
import type { tutorialDe } from "./tutorial.de";

export const tutorialRu: Fully<typeof tutorialDe> = {
  "tutorial.dialog": "Знакомство с Lorelamp",
  "tutorial.later": "Позже",
  "tutorial.back": "Назад",
  "tutorial.skip": "Пропустить",
  "tutorial.next": "Дальше",
  "tutorial.finish": "Готово",
  "tutorial.step": "Шаг {step} из {all}",
  "tutorial.steps": "Прогресс",
  "tutorial.failed": "Не получилось ({error}).",

  "tutorial.welcome.lead": "Здесь вы пишете истории вместе с персонажами, которые вам отвечают. Модель работает на вашем собственном ПК.",
  "tutorial.welcome.language": "Язык",
  "tutorial.welcome.start": "Начать",
  "tutorial.welcome.by": "Автор: {name}",
  "tutorial.welcome.code": "Исходный код на GitHub",
  "tutorial.welcome.again": "Пройти знакомство заново можно в любой момент: Настройки, Общие.",

  "tutorial.model.title": "Настройте модель",
  "tutorial.model.lead": "Модель пишет реплики ваших персонажей. Она работает у вас, без аккаунта и подписки.",
  "tutorial.model.ready": "Настроено: {name}",
  "tutorial.model.readySome": "Модель уже настроена.",
  "tutorial.model.todo": "Lorelamp предложит модели, которые подойдут вашему ПК. Скачивание займёт несколько минут, смотря по размеру.",
  "tutorial.model.atPc": "Модель настраивается на самом ПК. Здесь можно просто идти дальше.",

  "tutorial.figure.title": "Добавьте персонажа",
  "tutorial.figure.lead": "Персонажи приходят карточками в PNG или JSON, например из SillyTavern или с Chub. А можно собрать своего.",
  "tutorial.figure.there": { one: "У вас уже {n} персонаж.", few: "У вас уже {n} персонажа.", many: "У вас уже {n} персонажей.", other: "У вас уже {n} персонажа." },
  "tutorial.figure.import": "Импорт карточки",
  "tutorial.figure.build": "Создать персонажа",

  "tutorial.persona.title": "Кто вы в этой истории?",
  "tutorial.persona.lead": "По этому имени к вам будут обращаться персонажи. Внешность, характер и предысторию можно позже добавить в персону.",
  "tutorial.persona.name": "Ваше имя",
  "tutorial.persona.placeholder": "например, Алекс",
  "tutorial.persona.saved": "Сохранено.",
  "tutorial.persona.more": "Создать полную персону",

  "tutorial.chat.title": "Ваш первый чат",
  "tutorial.chat.lead": "Выберите, с кем начать. Персонаж заговорит первым, а вы ответите в поле внизу.",
  "tutorial.chat.none": "Как только появится персонаж, здесь одним касанием начнётся ваш первый чат.",
  "tutorial.chat.toFigure": "Добавить персонажа",
  "tutorial.chat.with": "Начать чат: {name}",
  "tutorial.chat.quiet": "Пока модель не запущена, ответа не будет. Но открыть чат уже можно.",

  "tutorial.dock": "Продолжить знакомство",

  "tutorial.settings.label": "Знакомство",
  "tutorial.settings.note": "Короткий тур первого запуска: язык, модель, персонаж, ваше имя, первый чат.",
  "tutorial.settings.restart": "Начать заново",
  "tutorial.cmd.label": "Пройти знакомство заново",
  "tutorial.cmd.keys": "туториал обучение знакомство помощь первые шаги приветствие tutorial tour onboarding",
};
