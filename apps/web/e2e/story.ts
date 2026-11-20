// A short, ordinary exchange with spoken words, *actions* and `thoughts`, for tests and review
// screenshots of how a chat looks (SFW, fixed text, so pictures do not depend on a model).

export const STORY: { role: "user" | "assistant"; content: string }[] = [
  { role: "user", content: "„Guten Morgen! Riecht es hier schon nach Zimt, oder bilde ich mir das ein?“" },
  {
    role: "assistant",
    content:
      "*Mara wischt sich das Mehl von den Händen und lehnt sich an die Theke.* „Du bildest dir gar nichts ein. Die Zimtschnecken brauchen noch zehn Minuten, dann sind sie perfekt.“\n\n`Hoffentlich bleibt er auf einen Kaffee.` *Sie schiebt dir einen Becher über das Holz und nickt zum Fenster hin, wo die Sonne gerade über die Dächer steigt.*",
  },
  { role: "user", content: "*Ich setze mich auf den Hocker am Fenster.* „Zehn Minuten schaffe ich. Was backst du sonst noch heute?“" },
  {
    role: "assistant",
    content:
      "„Roggenbrot für den Markt, zwei Bleche Apfelkuchen und, wenn die Zeit reicht, die kleinen Mandelhörnchen, nach denen alle fragen.“ *Sie zählt an den Fingern ab und lacht leise.* „Also eigentlich zu viel für einen Vormittag.“",
  },
  { role: "user", content: "„Brauchst du Hilfe? Ich kann wenigstens Teig kneten.“" },
  {
    role: "assistant",
    content: "*Mara mustert dich einen Moment, dann reicht sie dir eine Schürze.* „Angebot angenommen. Aber wer knetet, probiert zuerst.“",
  },
];
