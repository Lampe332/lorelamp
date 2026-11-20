import type { Fully } from "../types";
import type { tutorialDe } from "./tutorial.de";

export const tutorialEn: Fully<typeof tutorialDe> = {
  "tutorial.dialog": "Getting started with Lorelamp",
  "tutorial.later": "Later",
  "tutorial.back": "Back",
  "tutorial.skip": "Skip",
  "tutorial.next": "Next",
  "tutorial.finish": "Done",
  "tutorial.step": "Step {step} of {all}",
  "tutorial.steps": "Progress",
  "tutorial.failed": "That didn't work ({error}).",

  "tutorial.welcome.lead": "Tell stories with characters who answer back. The model behind them runs on your own PC.",
  "tutorial.welcome.language": "Language",
  "tutorial.welcome.start": "Let's go",
  "tutorial.welcome.by": "Made by {name}",
  "tutorial.welcome.code": "Source code on GitHub",
  "tutorial.welcome.again": "You can restart this tour any time under Settings, General.",

  "tutorial.model.title": "Set up a model",
  "tutorial.model.lead": "The model writes what your characters say. It runs on your machine. No account, no subscription.",
  "tutorial.model.ready": "Set up: {name}",
  "tutorial.model.readySome": "A model is set up.",
  "tutorial.model.todo": "Lorelamp suggests models that fit your PC. A download takes a few minutes, depending on its size.",
  "tutorial.model.atPc": "You set up the model on the PC itself. Here you can just carry on.",

  "tutorial.figure.title": "Get a character",
  "tutorial.figure.lead": "Characters come as cards, PNG or JSON, for example from SillyTavern or Chub. Or you build one yourself.",
  "tutorial.figure.there": { one: "You already have {n} character.", other: "You already have {n} characters." },
  "tutorial.figure.import": "Import card",
  "tutorial.figure.build": "Build a new character",

  "tutorial.persona.title": "Who are you in the story?",
  "tutorial.persona.lead": "Characters will call you by this name. Looks, personality and backstory can go into a persona later.",
  "tutorial.persona.name": "Your name",
  "tutorial.persona.placeholder": "for example Alex",
  "tutorial.persona.saved": "Saved.",
  "tutorial.persona.more": "Set up a full persona",

  "tutorial.chat.title": "Your first chat",
  "tutorial.chat.lead": "Pick someone. The character speaks first, you reply in the box at the bottom.",
  "tutorial.chat.none": "Once you have a character, you start your first chat here with one tap.",
  "tutorial.chat.toFigure": "Get a character",
  "tutorial.chat.with": "Start a chat with {name}",
  "tutorial.chat.quiet": "Until a model runs, no reply will come. You can still open the chat.",

  "tutorial.dock": "Back to the tour",

  "tutorial.settings.label": "Getting started",
  "tutorial.settings.note": "The short tour from the first start: language, model, character, your name, first chat.",
  "tutorial.settings.restart": "Start again",
  "tutorial.cmd.label": "Restart the tour",
  "tutorial.cmd.keys": "tutorial tour getting started guide help first steps welcome onboarding",
};
