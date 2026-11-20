import type { Fully } from "../types";
import type { tutorialDe } from "./tutorial.de";

export const tutorialEs: Fully<typeof tutorialDe> = {
  "tutorial.dialog": "Primeros pasos en Lorelamp",
  "tutorial.later": "Más tarde",
  "tutorial.back": "Atrás",
  "tutorial.skip": "Saltar",
  "tutorial.next": "Siguiente",
  "tutorial.finish": "Listo",
  "tutorial.step": "Paso {step} de {all}",
  "tutorial.steps": "Progreso",
  "tutorial.failed": "No ha funcionado ({error}).",

  "tutorial.welcome.lead": "Aquí cuentas historias con personajes que te responden. El modelo que hay detrás funciona en tu propio PC.",
  "tutorial.welcome.language": "Idioma",
  "tutorial.welcome.start": "Empezar",
  "tutorial.welcome.by": "Hecho por {name}",
  "tutorial.welcome.code": "Código fuente en GitHub",
  "tutorial.welcome.again": "Puedes repetir esta introducción cuando quieras en Ajustes, General.",

  "tutorial.model.title": "Configura un modelo",
  "tutorial.model.lead": "El modelo escribe lo que dicen tus personajes. Funciona en tu equipo, sin cuenta y sin suscripción.",
  "tutorial.model.ready": "Configurado: {name}",
  "tutorial.model.readySome": "Ya hay un modelo configurado.",
  "tutorial.model.todo": "Lorelamp te sugiere modelos que caben en tu PC. La descarga tarda unos minutos, según el tamaño.",
  "tutorial.model.atPc": "El modelo se configura en el PC. Aquí puedes seguir sin más.",

  "tutorial.figure.title": "Consigue un personaje",
  "tutorial.figure.lead": "Los personajes vienen como tarjetas, en PNG o JSON, por ejemplo de SillyTavern o de Chub. O te creas uno tú.",
  "tutorial.figure.there": { one: "Ya tienes {n} personaje.", other: "Ya tienes {n} personajes." },
  "tutorial.figure.import": "Importar tarjeta",
  "tutorial.figure.build": "Crear un personaje nuevo",

  "tutorial.persona.title": "¿Quién eres en la historia?",
  "tutorial.persona.lead": "Con este nombre te hablarán los personajes. Tu aspecto, tu carácter y tu pasado puedes añadirlos más tarde en una persona.",
  "tutorial.persona.name": "Tu nombre",
  "tutorial.persona.placeholder": "por ejemplo, Alex",
  "tutorial.persona.saved": "Guardado.",
  "tutorial.persona.more": "Crear una persona completa",

  "tutorial.chat.title": "Tu primer chat",
  "tutorial.chat.lead": "Elige a alguien. El personaje empieza y tú contestas en el campo de abajo.",
  "tutorial.chat.none": "En cuanto tengas un personaje, aquí empiezas tu primer chat con un toque.",
  "tutorial.chat.toFigure": "Conseguir un personaje",
  "tutorial.chat.with": "Empezar un chat con {name}",
  "tutorial.chat.quiet": "Mientras no funcione ningún modelo, no llegará respuesta. Aun así puedes abrir el chat.",

  "tutorial.dock": "Seguir con la introducción",

  "tutorial.settings.label": "Introducción",
  "tutorial.settings.note": "El recorrido corto del primer inicio: idioma, modelo, personaje, tu nombre y el primer chat.",
  "tutorial.settings.restart": "Empezar de nuevo",
  "tutorial.cmd.label": "Repetir la introducción",
  "tutorial.cmd.keys": "tutorial introducción guía ayuda primeros pasos bienvenida tour onboarding",
};
