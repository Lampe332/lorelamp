/* RP-App – klickbarer UI-Prototyp. Statisch, kein Netzwerk, keine echte KI.
   Hash-Routen: #/erststart/1..5 · #/chats · #/chat/<id> · #/charaktere · #/charakter/<id>/<tab>
                #/entdecken/<tab> · #/einstellungen/<seite>
   Alle Inhalte sind erfundene Beispieldaten. */
'use strict';

/* ================= Hilfen ================= */
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const ic = (id, cls = '') => `<svg class="ic ${cls}" aria-hidden="true"><use href="#i-${id}"/></svg>`;
const fmt = n => n.toLocaleString('de-DE');
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const tok = s => Math.max(0, Math.round(String(s).length / 3.6));
const md = t => esc(t).split(/\n{2,}/).map(p => `<p>${p.replace(/\*(.+?)\*/g, '<em>$1</em>').replace(/\n/g, '<br>')}</p>`).join('');
const store = {
  get(k) { try { return localStorage.getItem(k); } catch (e) { return null; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch (e) { /* ohne Speicher weiter */ } },
};

/* ================= Daten ================= */
const CHARS = [
  { id: 'maren', name: 'Maren Holt', sub: 'Kapitänin der Sturmschwalbe', age: 34, tags: ['Abenteuer', 'Seefahrt', 'Slow Burn'], fav: true, tone: '#2b3d44', tone2: '#4f7180', motif: 'wave', tokens: 3058, chats: 2 },
  { id: 'ilvaris', name: 'Ilvaris Telmeren', sub: 'Archivarin der Mondbibliothek', age: 212, tags: ['Fantasy', 'Elfen', 'Mysterium'], fav: true, tone: '#22392f', tone2: '#5f8a72', motif: 'moon', tokens: 2988, chats: 1 },
  { id: 'vera', name: 'Vera Lind', sub: 'Inspektorin, Hamburg 1928', age: 38, tags: ['Krimi', 'Noir', 'Historisch'], fav: false, tone: '#35302d', tone2: '#7a6d64', motif: 'rain', tokens: 2104, chats: 1 },
  { id: 'nadja', name: 'Nadja Kovač', sub: 'Barkeeperin im Raumhafen Kessel-7', age: 31, tags: ['Sci-Fi', 'Slice of Life'], fav: false, tone: '#42262a', tone2: '#9a5a5f', motif: 'glass', tokens: 1650, chats: 1 },
  { id: 'selim', name: 'Dr. Selim Arslan', sub: 'Physiker mit Zeitproblem', age: 45, tags: ['Sci-Fi', 'Komödie'], fav: false, tone: '#3d3522', tone2: '#a08550', motif: 'gear', tokens: 1388, chats: 1 },
  { id: 'henrike', name: 'Henrike Sand', sub: 'Leuchtturmwärterin auf Grauhorn', age: 41, tags: ['Gemütlich', 'Drama'], fav: false, tone: '#283540', tone2: '#c9a764', motif: 'beam', tokens: 1802, chats: 1 },
  { id: 'ravel', name: 'Ravel Darcourt', sub: 'Gentleman-Dieb mit Prinzipien', age: 36, tags: ['Abenteuer', 'Intrige'], fav: false, tone: '#333626', tone2: '#7f8657', motif: 'key', tokens: 2230, chats: 0 },
  { id: 'moos', name: 'Moos', sub: 'Waldgeist am Grenzpfad', age: null, tags: ['Fantasy', 'Gemütlich'], fav: false, tone: '#2c3824', tone2: '#7c9a5c', motif: 'leaf', tokens: 980, chats: 0 },
];
const charById = id => CHARS.find(c => c.id === id) || CHARS[0];
const PERSONA = { name: 'Lukas', sub: 'Kartograf', tone: '#3a342e', tone2: '#6d645a', motif: 'none' };

const CHATS = [
  { id: 'c1', char: 'maren', title: 'Sturm vor Kap Morn', last: 'Und, Lukas? Ziehen Sie die nassen Stiefel aus, bevor Sie mir die Kajüte ruinieren.', when: 'vor 12 Min', msgs: 142, tk: '34k', pinned: true },
  { id: 'c2', char: 'ilvaris', title: 'Das verbotene Register', last: 'Sie schließt das Buch, ohne hinzusehen. „Manche Seiten lesen zurück.“', when: 'gestern', msgs: 88, tk: '21k', pinned: true },
  { id: 'c3', char: 'vera', title: 'Der Fall Elbbrücke', last: '„Drei Zeugen, drei Uhrzeiten. Einer davon lügt – und es ist nicht der Betrunkene.“', when: 'gestern', msgs: 61, tk: '15k', pinned: false },
  { id: 'c4', char: 'nadja', title: 'Letzte Runde', last: 'Sie schiebt dir ein Glas hin, das leise leuchtet. „Geht aufs Haus. Frag nicht, was drin ist.“', when: 'Mo.', msgs: 23, tk: '6k', pinned: false },
  { id: 'c5', char: 'selim', title: 'Dienstag, schon wieder', last: '„Wenn das hier der vierte Dienstag ist, dann haben Sie mir das schon dreimal erzählt.“', when: '08.11.', msgs: 9, tk: '2k', pinned: false },
  { id: 'c6', char: 'maren', title: 'Hafen von Velde', last: 'Die Taue sind fest, die Mannschaft an Land. Nur die Kapitänin bleibt an der Reling stehen.', when: '02.11.', msgs: 204, tk: '58k', pinned: false },
  { id: 'c7', char: 'henrike', title: 'Nebelnacht', last: 'Das Licht dreht sich weiter, egal wie dicht der Nebel ist. „Es hat noch nie aufgehört.“', when: '30.10.', msgs: 37, tk: '9k', pinned: false },
];

const LAST_VARIANTS = [
  `*Sie mustert dich lange, dann rollt sie die Karte mit einer einzigen Bewegung zusammen.*\n\n„Bei Sonnenaufgang also.“ *Sie klemmt die Rolle unter den Arm.* „Schlafen Sie, Kartograf. Ich wecke Sie, wenn es Zeit ist, sich zu irren.“`,
  `*Draußen schlägt eine Welle gegen den Rumpf, und die Laterne kippt so weit, dass das Licht für einen Herzschlag nur ihr Gesicht trifft. Sie lacht – kurz, überrascht, als hätte sie vergessen, wie das geht.*\n\n„Bei Sonnenaufgang.“ *Sie richtet sich auf, greift nach dem Ölzeug am Haken und wirft es dir zu.* „Gut. Aber wenn wir das machen, stehen Sie neben mir am Ruder und lesen mir jede Zahl auf dieser Karte vor. Laut. Ich will Ihre Stimme hören, wenn Sie sich irren.“\n\n*An der Tür bleibt sie stehen, eine Hand am Rahmen, den Blick noch einmal auf dich gerichtet – länger, als eine Kapitänin einen Passagier ansehen müsste.*\n\n„Und, Lukas? Ziehen Sie die nassen Stiefel aus, bevor Sie mir die Kajüte ruinieren. Sie gehören jetzt zur Mannschaft.“`,
  `„Mutig also.“ *Sie lehnt sich gegen den Kartentisch, die Arme verschränkt.* „Mein Vater hat auch an alte Karten geglaubt. Er hat die Sturmschwalbe zweimal fast verloren.“\n\n*Eine Pause. Dann, leiser:* „Zeigen Sie mir noch einmal die Tiefenangaben.“`,
  `*Maren antwortet nicht sofort. Sie geht zum Bullauge, wischt mit dem Ärmel über das beschlagene Glas und starrt in die Dunkelheit, wo irgendwo die Klippen sein müssen.*\n\n„Okke wird mich für verrückt halten“, *sagt sie schließlich.* „Gut. Dann sind wir schon zwei.“`,
  `„Sonnenaufgang ist in fünf Stunden.“ *Sie zieht eine Taschenuhr hervor, klappt sie auf, wieder zu.* „Das reicht für Tee, für Ihre Karte und für eine sehr gute Ausrede, falls Sie falschliegen.“`,
];

const INITIAL_MSGS = () => ([
  { id: 'm1', who: 'char', time: '20:41', text: `*Die Laterne über dem Kartentisch schwingt im Takt der Wellen, und ihr Licht wandert über Seekarten, die längst niemand mehr glatt gestrichen hat. Maren Holt steht mit dem Rücken zu dir, die Hände auf der Tischkante, als könnte sie das Schiff allein mit ihrem Gewicht ruhig halten.*\n\n„Sie sind also der Kartograf, für den die Gesellschaft so viel bezahlt hat.“ *Sie dreht sich um und mustert dich vom nassen Mantel bis zu den Stiefeln.* „Dann erklären Sie mir mal, warum Ihre Karte eine Bucht zeigt, wo seit dreißig Jahren nur Klippen sind.“`, sw: 0, swN: 3, tokens: 168, tps: 103.2 },
  { id: 'm2', who: 'user', time: '20:43', text: `„Weil die Karte nicht von heute ist. Sie ist von 1791 – und damals war da eine Einfahrt.“ *Ich lege die Rolle neben ihre Hand und streiche sie vorsichtig glatt.*`, tokens: 46 },
  { id: 'm3', who: 'char', time: '20:44', text: `*Sie beugt sich über das Pergament, so nah, dass ihre Schulter fast deine streift. Für einen Moment sagt sie nichts; nur das Knarren der Spanten füllt die Kajüte.*\n\n„Siebzehnhunderteinundneunzig.“ *Ein schiefes Lächeln.* „Sie wissen schon, dass Sie gerade vorschlagen, mein Schiff durch eine Einfahrt zu steuern, die es vielleicht gar nicht mehr gibt? Bei Nacht. Bei diesem Wetter.“\n\n*Sie tippt auf eine verblasste Tiefenangabe.* „Mutig. Oder dumm. Ich habe noch nicht entschieden, was mir an Ihnen besser gefällt.“`, sw: 0, swN: 1, tokens: 141, tps: 104.6 },
  { id: 'm4', who: 'user', time: '20:46', text: `„Entscheiden Sie bei Sonnenaufgang. Wenn wir dann noch schwimmen, war es mutig.“`, tokens: 24 },
  { id: 'm5', who: 'char', time: '20:46', variants: LAST_VARIANTS.slice(), sw: 1, swN: 5, think: 'Maren testet ihn noch, aber der Mut gefällt ihr. Spannung halten, der Sturm gibt das Tempo vor. Nicht für Lukas handeln – nur ihr Angebot, ihre Geste an der Tür.', thinkMeta: '1,4 s · 58 Tokens', tokens: 214, tps: 104.1 },
]);

const SUGGESTIONS = [
  `„Aye, Kapitänin.“ *Ich ziehe die Stiefel aus und folge ihr an Deck.*`,
  `„Nur wenn Sie mir sagen, warum Sie wirklich Ja gesagt haben.“`,
  `*Ich halte sie am Ärmel zurück.* „Warten Sie – die Karte hat noch eine zweite Markierung.“`,
];
const IMPERSONATE = `„Zu Befehl.“ *Ich fange das Ölzeug auf, grinse und streife mir die Stiefel ab.* „Aber wenn ich recht habe, schulden Sie mir einen Tee. Mit Rum.“`;
const FIX_TEXT = {
  'Kürzer': `*Sie lacht kurz und wirft dir das Ölzeug zu.* „Bei Sonnenaufgang. Sie stehen neben mir am Ruder – und ziehen Sie die nassen Stiefel aus. Sie gehören jetzt zur Mannschaft.“`,
  'Länger': LAST_VARIANTS[1] + `\n\n*Ihre Schritte verklingen auf dem Niedergang. Oben ruft jemand nach dem Bootsmann, Segeltuch knallt im Wind, und durch die Planken spürst du, wie die Sturmschwalbe langsam ihren Kurs ändert – auf die Klippen zu.*`,
  'Mehr Dialog': `„Bei Sonnenaufgang.“ *Sie lacht.* „Wissen Sie, was mein Vater gesagt hätte? ‚Trau keinem Mann, der trocken an Bord kommt.‘ Sie sind klatschnass. Das spricht für Sie.“\n\n„Und wenn ich mich irre?“\n\n„Dann irren Sie sich laut, neben mir am Ruder, damit ich es rechtzeitig höre.“ *Sie wirft dir das Ölzeug zu.* „Stiefel aus, Kartograf. Sie gehören jetzt zur Mannschaft.“`,
  'Weniger blumig': `*Eine Welle trifft den Rumpf. Maren lacht kurz.*\n\n„Gut. Bei Sonnenaufgang.“ *Sie wirft dir das Ölzeug zu.* „Sie stehen dann neben mir am Ruder und lesen mir die Zahlen vor. Und ziehen Sie die nassen Stiefel aus. Sie gehören jetzt zur Mannschaft.“`,
  'Anders': LAST_VARIANTS[3],
};
const REPLY_TEXT = `*Maren fängt deinen Blick auf, und für einen Moment scheint der Sturm draußen leiser zu werden. Dann schnaubt sie, halb amüsiert, halb geschlagen.*\n\n„Einen Tee. Mit Rum.“ *Sie schüttelt den Kopf.* „Sie verhandeln wie ein Schmuggler, Kartograf. Abgemacht – aber nur, wenn wir morgen Abend noch einen Tisch haben, an dem wir ihn trinken können.“\n\n*Sie stößt die Tür auf. Regen weht herein, kalt und salzig, und sie tritt hinaus, ohne sich noch einmal umzudrehen. Die Tür bleibt offen.*`;

const LOREBOOKS = [
  { name: 'Sturmschwalbe – Schiff & Crew', n: 14, act: 2, on: true },
  { name: 'Küste von Morn', n: 9, act: 1, on: true },
  { name: 'Seemannsgarn (global)', n: 22, act: 0, on: false },
];
const LORE_ENTRIES = [
  { keys: ['Sturmschwalbe', 'Schiff'], text: 'Dreimast-Schoner, 1789 gebaut, Marens Erbe vom Vater. Vierzehn Mann Besatzung.', pos: 'Schwanz', tk: 96 },
  { keys: ['Kap Morn', 'Klippen'], text: 'Gefürchtete Landspitze; seit dem Bergsturz von 1802 gilt die alte Einfahrt als verschüttet.', pos: 'Schwanz', tk: 84 },
  { keys: ['Okke', 'Bootsmann'], text: 'Okke Brandt, 58, Bootsmann. Loyal zu Maren, misstraut Kartografen grundsätzlich.', pos: 'Schwanz', tk: 72 },
  { keys: ['Kajüte', 'Laterne'], text: 'Kartentisch, Messinglaterne, eine Flasche Rum, die Maren nie anrührt.', pos: 'Schwanz', tk: 58 },
  { keys: ['(immer aktiv)'], text: 'Zeit: um 1821. Keine moderne Technik, Navigation per Sextant und Lot.', pos: 'Kopf · Dauer', tk: 40, always: true },
];

/* Prompt-Inspektor: eine Quelle für alle Zahlen */
const PROMPT = {
  head: [['Basis-Systemprompt', 612], ['Persona: Lukas', 148], ['Karte: Beschreibung · Persönlichkeit · Szenario', 1056], ['Dauer-Lorebook (1 Eintrag)', 40], ['Beispieldialoge', 960]],
  tail: [['Ausgelöste Lorebook-Einträge (2)', 156], ["Author's Note", 64], ['Intensitäts-Hinweis (Stufe 3)', 38], ['Regie-Anweisung', 0], ['Post-History der Karte', 120], ['Antwort-Anfang „Maren:“', 6]],
  total: 34012,
  ctx: 131072,
  fresh: 1020, // neu berechnet: Schwanz + die zwei neuen Züge
};
const sum = a => a.reduce((s, r) => s + r[1], 0);
PROMPT.headSum = sum(PROMPT.head);
PROMPT.tailSum = sum(PROMPT.tail);
PROMPT.histSum = PROMPT.total - PROMPT.headSum - PROMPT.tailSum;
PROMPT.cache = Math.round((1 - PROMPT.fresh / PROMPT.total) * 100);
console.assert(PROMPT.headSum + PROMPT.histSum + PROMPT.tailSum === PROMPT.total, 'Prompt-Blöcke müssen die Summe ergeben');
console.assert(PROMPT.cache === 97, 'Cache-Quote muss zur Statuspille passen');

const MODELS = {
  installed: [
    { id: 'orion', name: 'Orion 26B-A4B', q: 'Q4_K_M', gb: 15.2, note: '26B (4B aktiv) · passt mit 128k', where: 'LM-Studio-Ordner', active: true },
    { id: 'qwen', name: 'Qwen3.8 14B', q: 'IQ3_S', gb: 6.9, note: '14B · passt mit 128k', where: 'App-Modellordner' },
  ],
  loading: { name: 'Mistral Small 3.2 24B', q: 'Q4_K_M', gb: 14.3, done: 8.7, speed: 48 },
  recommended: [
    { name: 'Gemma 4 12B', q: 'Q8_0', gb: 12.6, note: 'schnell, gutes Deutsch · 128k' },
    { name: 'Nemo-RP 12B', q: 'Q6_K', gb: 10.1, note: 'Rollenspiel-Feinschliff · 64k' },
    { name: 'Llama 3.3 8B', q: 'Q8_0', gb: 8.5, note: 'klein, für Nebenbei-Betrieb · 128k' },
  ],
};
const TUNE = [
  { be: 'ROCm', kv: 'q8_0', g0: 104, g64: 71, pp: 2950, ctx: '128k', vram: 21.4, best: true },
  { be: 'ROCm', kv: 'f16', g0: 106, g64: 73, pp: 3010, ctx: '64k', vram: 22.8 },
  { be: 'Vulkan', kv: 'q8_0', g0: 66, g64: 58, pp: 1410, ctx: '128k', vram: 21.9 },
  { be: 'Vulkan', kv: 'f16', g0: 68, g64: 60, pp: 1460, ctx: '64k', vram: 23.1 },
];

const LVL = { 1: 'Einfach', 2: 'Erweitert', 3: 'Experte' };
const SETTINGS = [
  { id: 'allgemein', title: 'Allgemein', groups: [
    { title: 'Darstellung', rows: [
      { key: 'theme', lvl: 1, name: 'Hell oder Dunkel', ex: 'Dunkel schont die Augen beim Lesen am Abend.', type: 'seg', opts: ['Dunkel', 'Hell', 'System'], def: 'Dunkel' },
      { key: 'fontsize', lvl: 1, name: 'Schriftgröße im Chat', term: 'px', ex: 'Größe der Erzähltexte. Die Oberfläche bleibt gleich.', type: 'range', min: 15, max: 22, step: 1, def: 18 },
      { key: 'uilang', lvl: 1, name: 'Sprache der Oberfläche', ex: 'Menüs und Erklärungen. Die Antwortsprache stellst du unter „Chat“ ein.', type: 'select', opts: ['Deutsch', 'English'], def: 'Deutsch' },
      { key: 'width', lvl: 2, name: 'Breite der Chat-Spalte', ex: 'Schmalere Zeilen lesen sich ruhiger, breitere zeigen mehr auf einmal.', type: 'seg', opts: ['Schmal', 'Mittel', 'Breit'], def: 'Mittel' },
    ] },
    { title: 'Über dich', rows: [
      { key: 'username', lvl: 1, name: 'Dein Name', term: '{{user}}', ex: 'So sprechen dich die Charaktere an, wenn keine Persona gewählt ist.', type: 'text', def: 'Lukas' },
    ] },
  ] },
  { id: 'chat', title: 'Chat', groups: [
    { title: 'Antworten', rows: [
      { key: 'len', lvl: 1, name: 'Antwortlänge', term: 'max. Tokens', ex: 'Wie lang eine Antwort höchstens wird. Kurz ≈ 180, Mittel ≈ 350, Lang ≈ 700 Tokens.', type: 'seg', opts: ['Kurz', 'Mittel', 'Lang'], def: 'Mittel' },
      { key: 'style', lvl: 1, name: 'Stil', term: 'Sampler-Preset', ex: 'Präzise bleibt eng an der Szene, Wild probiert mehr aus – und verliert öfter den Faden.', type: 'seg', opts: ['Präzise', 'Ausgewogen', 'Kreativ', 'Wild'], def: 'Ausgewogen' },
      { key: 'answerlang', lvl: 1, name: 'Antwortsprache', ex: 'In dieser Sprache schreiben die Charaktere – unabhängig von der Karte.', type: 'select', opts: ['Deutsch', 'Englisch', 'Wie die Karte'], def: 'Deutsch' },
      { key: 'pov', lvl: 1, name: 'Erzählform', ex: 'Aus welcher Sicht die Erzählung geschrieben wird. Beispiel „Du“: „Du spürst den Regen.“', type: 'seg', opts: ['Ich', 'Du', 'Er/Sie'], def: 'Du' },
      { key: 'intensity', lvl: 1, name: 'Intensität', ex: 'Aus, per Regler in der Szene-Leiste, oder automatisch steigend mit dem Verlauf. Du kannst jederzeit runter.', type: 'seg', opts: ['Aus', 'Manuell', 'Auto'], def: 'Manuell' },
      { key: 'suggest', lvl: 1, name: 'Vorschläge anzeigen', ex: 'Drei Ideen für deinen nächsten Zug über dem Eingabefeld. Antippen setzt sie ins Feld, nichts wird sofort gesendet.', type: 'toggle', def: true },
    ] },
    { title: 'Sampler', desc: 'Feineinstellung, wie das Modell Wörter wählt. Die Presets oben setzen diese Werte für dich.', rows: [
      { key: 'temp', lvl: 2, name: 'Kreativität', term: 'Temperature', ex: 'Höher = überraschender, niedriger = vorhersehbarer. 0,8–1,0 passt für die meisten Rollenspiele.', type: 'range', min: 0, max: 2, step: 0.05, def: 0.9 },
      { key: 'minp', lvl: 2, name: 'Untergrenze für Wortwahl', term: 'Min-P', ex: 'Schneidet sehr unwahrscheinliche Wörter ab. Verhindert Wortsalat bei hoher Kreativität.', type: 'range', min: 0, max: 0.3, step: 0.01, def: 0.05 },
      { key: 'dry', lvl: 2, name: 'Wiederholungs-Bremse', term: 'DRY-Multiplikator', ex: 'Bremst Sätze, die sich wortgleich wiederholen. 0 = aus.', type: 'range', min: 0, max: 2, step: 0.1, def: 0.8 },
      { key: 'xtc', lvl: 2, name: 'Floskel-Ausweichen', term: 'XTC-Wahrscheinlichkeit', ex: 'Lässt das Modell manchmal die naheliegendste Formulierung auslassen. Gegen „Ein Schauer lief ihr über den Rücken“.', type: 'range', min: 0, max: 1, step: 0.05, def: 0.1 },
      { key: 'ctx', lvl: 2, name: 'Kontextgröße', term: 'n_ctx', ex: 'Wie viel vom Chat das Modell auf einmal sieht. Mehr braucht mehr Grafikspeicher.', type: 'select', opts: ['32k', '64k', '128k'], def: '128k' },
      { key: 'reason', lvl: 2, name: 'Denken vor der Antwort', term: 'Reasoning', ex: 'Das Modell plant kurz, bevor es schreibt. Langsamer, manchmal stimmiger. Der Denkblock bleibt eingeklappt.', type: 'toggle', def: false },
    ] },
    { title: 'Prompt', rows: [
      { key: 'sysprompt', lvl: 2, name: 'Systemprompt', ex: 'Grundregeln fürs Erzählen.', type: 'textarea', def: 'Du bist ein Erzähler in einem fortlaufenden Rollenspiel. Zeige statt zu erzählen. Schreibe nie Handlungen oder Worte für {{user}}. Vermeide Wiederholungen.', wide: true },
      { key: 'lorebudget', lvl: 2, name: 'Lorebook-Budget', term: '% des Kontexts', ex: 'Wie viel Platz Lorebook-Einträge höchstens bekommen.', type: 'range', min: 5, max: 40, step: 1, def: 15 },
      { key: 'scan', lvl: 2, name: 'Lorebook-Suchtiefe', term: 'Nachrichten', ex: 'In wie vielen der letzten Nachrichten nach Schlüsselwörtern gesucht wird.', type: 'range', min: 1, max: 12, step: 1, def: 4 },
      { key: 'trim', lvl: 2, name: 'Unfertigen Satz abschneiden', ex: 'Entfernt einen halben Satz am Ende, wenn die Längengrenze zuschlägt.', type: 'toggle', def: true },
      { key: 'template', lvl: 3, name: 'Template-Override', term: 'Chat-Template', ex: 'Normalerweise kommt das Template exakt aus der Modelldatei. Nur ändern, wenn du weißt warum.', type: 'code', def: '(automatisch aus GGUF: gemma4)', wide: true },
      { key: 'samplerjson', lvl: 3, name: 'Freies Sampler-JSON', ex: 'Alles, was es oben nicht gibt. Wird zuletzt angewendet.', type: 'code', def: '{ "top_k": 0, "typical_p": 1.0, "dry_allowed_length": 3 }', wide: true },
      { key: 'logit', lvl: 3, name: 'Wort-Gewichte', term: 'Logit-Bias', ex: 'Einzelne Wörter bevorzugen oder verbieten.', type: 'code', def: '{ "Schauer": -4, "Hauch": -2 }', wide: true },
    ] },
  ] },
  { id: 'modell', title: 'Modell & Leistung', groups: [
    { title: 'Modell', rows: [
      { key: 'model', lvl: 1, name: 'Modell', ex: 'Das Sprachmodell, mit dem alle Chats laufen. Gefunden im LM-Studio-Ordner.', type: 'select', opts: ['Orion 26B-A4B · Q4_K_M', 'Qwen3.8 14B · IQ3_S'], def: 'Orion 26B-A4B · Q4_K_M' },
      { key: 'unload', lvl: 1, name: 'Im Leerlauf entladen', ex: 'Gibt den Grafikspeicher frei, wenn du eine Weile nicht chattest. Praktisch, wenn du nebenbei Bilder generierst.', type: 'seg', opts: ['Aus', '10 Min', '30 Min'], def: 'Aus' },
      { key: 'awake', lvl: 1, name: 'PC wach halten', ex: 'Verhindert Standby, solange generiert wird oder ein Handy verbunden ist.', type: 'toggle', def: true },
    ] },
    { title: 'Engine', rows: [
      { key: 'engine', lvl: 2, name: 'Rechenweg', term: 'Backend', ex: 'Auto-Tune hat ROCm auf deiner Karte als schnellsten gemessen (104 statt 66 tok/s).', type: 'seg', opts: ['ROCm', 'Vulkan', 'CPU'], def: 'ROCm' },
      { key: 'kv', lvl: 2, name: 'Speicher für den Kontext', term: 'KV-Cache', ex: 'q8_0 braucht halb so viel Grafikspeicher wie f16 – dadurch passen 128k auf 24 GB.', type: 'seg', opts: ['q8_0', 'f16'], def: 'q8_0' },
      { key: 'batch', lvl: 2, name: 'Einlese-Paketgröße', term: '-b / -ub', ex: 'Größere Pakete lesen lange Chats schneller ein, brauchen etwas mehr Speicher.', type: 'select', opts: ['2048 / 512', '4096 / 1024', '1024 / 256'], def: '2048 / 512' },
      { key: 'free', lvl: 2, name: 'Andere Programme freigeben', ex: 'ComfyUI belegt gerade 2,1 GB Grafikspeicher. Ein Klick bittet es, den Speicher zu leeren.', type: 'button', label: 'ComfyUI freigeben', def: null },
      { key: 'flags', lvl: 3, name: 'Freie Engine-Flags', term: 'llama-server', ex: 'Werden beim Start angehängt. Falsche Flags verhindern den Start – der letzte gute Stand wird dann geladen.', type: 'text', def: '--ctx-checkpoints 8 --cache-reuse 256', wide: true },
    ] },
  ] },
  { id: 'handy', title: 'Handy & WLAN', custom: 'handy', groups: [] },
  { id: 'daten', title: 'Daten & Backup', groups: [
    { title: 'Speicherort', rows: [
      { key: 'datadir', lvl: 1, name: 'Datenordner', ex: 'Chats, Karten und Backups. Gewählt beim Erststart: das Laufwerk mit dem meisten Platz.', type: 'text', def: 'D:\\RP-App', },
      { key: 'backup', lvl: 1, name: 'Automatische Backups', ex: 'Täglich eine Kopie aller Chats und Karten. Die letzten 7 bleiben erhalten.', type: 'toggle', def: true },
      { key: 'kvdisk', lvl: 2, name: 'Kontext auf Platte merken', term: 'Slot-Speicher', ex: 'Lange Chats öffnen sofort, statt neu eingelesen zu werden. Ältestes wird zuerst gelöscht.', type: 'select', opts: ['Aus', '10 GB', '20 GB', '50 GB'], def: '20 GB' },
      { key: 'backupnow', lvl: 1, name: 'Jetzt sichern', ex: 'Letztes Backup: heute, 03:00 · 412 MB.', type: 'button', label: 'Backup erstellen', def: null },
    ] },
    { title: 'Umzug', rows: [
      { key: 'stimport', lvl: 1, name: 'SillyTavern-Ordner übernehmen', ex: 'Karten, Chats, Personas und Lorebooks mit einem Klick. Dein SillyTavern bleibt unverändert.', type: 'button', label: 'Ordner wählen …', def: null },
    ] },
  ] },
  { id: 'sicherheit', title: 'Sicherheit', groups: [
    { title: 'Netzwerk', rows: [
      { key: 'extimg', lvl: 1, name: 'Externe Bilder in Karten blockieren', ex: 'Bilder von fremden Servern könnten deine IP-Adresse verraten.', type: 'toggle', def: true },
      { key: 'token', lvl: 3, name: 'Zugangs-Token erneuern', ex: 'Alle gekoppelten Geräte müssen danach neu gekoppelt werden.', type: 'button', label: 'Neu erzeugen', def: null },
    ] },
  ] },
  { id: 'ueber', title: 'Über & Updates', groups: [
    { title: 'Version', rows: [
      { key: 'ver', lvl: 1, name: 'RP-App 0.1.0 (Prototyp)', ex: 'Lizenz AGPL-3.0 · Quellcode auf GitHub · keine Telemetrie, keine Konten.', type: 'info', value: 'Aktuell' },
      { key: 'upd', lvl: 1, name: 'Nach Updates suchen', ex: 'Einzige automatische Online-Verbindung. Abschaltbar.', type: 'toggle', def: true },
      { key: 'eng', lvl: 2, name: 'Engine', term: 'llama.cpp', ex: 'Windows-ROCm-Build vom 10.11.2026, Prüfsumme geprüft.', type: 'info', value: 'Aktuell' },
      { key: 'wizard', lvl: 1, name: 'Erststart noch einmal zeigen', ex: 'Führt durch Sprache, Grafikkarte, Modell und Auto-Tune. Deine Daten bleiben.', type: 'link', label: 'Erststart öffnen', href: '#/erststart/1', def: null },
    ] },
  ] },
  { id: 'experte', title: 'Experte', groups: [
    { title: 'Diagnose', rows: [
      { key: 'rawprompt', lvl: 3, name: 'Roh-Prompt des letzten Zugs', ex: 'Genau der Text, der an die Engine ging – mit Template-Zeichen.', type: 'code', def: '<start_of_turn>system\nDu bist ein Erzähler …<end_of_turn>\n<start_of_turn>user\n„Entscheiden Sie bei Sonnenaufgang …“<end_of_turn>\n<start_of_turn>model\nMaren:', wide: true },
      { key: 'regex', lvl: 3, name: 'Regex-Skripte', ex: 'Ersetzen Text in Antworten, bevor er angezeigt wird. 2 aktiv.', type: 'code', def: '/\\s*\\(OOC:.*?\\)//g\n/ {2,}/ /g', wide: true },
      { key: 'logs', lvl: 3, name: 'Engine-Log', ex: 'Letzte Zeilen von llama-server.', type: 'code', def: 'slot 0 | prompt 34012 tok, cached 32992 (97.0 %)\nslot 0 | eval 214 tok, 104.1 tok/s\nslot 0 | ctx-checkpoint saved (8/8)', wide: true },
    ] },
  ] },
];
const ALL_ROWS = SETTINGS.flatMap(p => p.groups.flatMap(g => g.rows.map(r => ({ ...r, page: p.id, pageTitle: p.title }))));

/* ================= Zustand ================= */
const S = {
  theme: document.documentElement.dataset.theme || 'dark',
  msgs: INITIAL_MSGS(),
  story: false,
  scene: true,
  sceneOpen: false,
  intensity: 3, intMode: 'Manuell', length: 'Mittel',
  lb: LOREBOOKS.map(l => l.on),
  regie: false, regieText: '',
  streaming: null,
  editing: null,
  info: null,
  chatQuery: '', chatTag: 'Alle',
  charQuery: '', charFilter: 'Alle',
  favs: Object.fromEntries(CHARS.map(c => [c.id, c.fav])),
  level: Number(store.get('rp-level')) || 1,
  setQuery: '',
  set: { wifi: true, ...Object.fromEntries(ALL_ROWS.filter(r => r.def !== undefined && r.def !== null).map(r => [r.key, r.def])) },
  modelSel: 'orion', loadPct: 61, loadPaused: false,
  greet: 0,
  loreTest: 'Wir halten auf Kap Morn zu, Okke soll die Segel reffen.',
  wiz: { lang: 'Deutsch', drive: 'D', eng: 0, model: 'orion', start: 'beispiel', tuneDone: true },
  devices: [
    { name: 'Galaxy S22', sub: 'Chrome · Stufe A · zuletzt vor 3 Min', icon: 'phone' },
    { name: 'Galaxy Tab S9', sub: 'Samsung Internet · Stufe B · zuletzt 08.11.', icon: 'tablet' },
  ],
};
const INT_NAMES = ['Ruhig', 'Locker', 'Warm', 'Nah', 'Intensiv', 'Maximal'];

/* ================= Avatare (flach, aus Formen) ================= */
function motif(m, c2) {
  switch (m) {
    case 'wave': return `<path d="M0 66q12.5-10 25 0t25 0 25 0 25 0v34H0z" fill="${c2}"/><path d="M0 80q12.5-8 25 0t25 0 25 0 25 0v20H0z" fill="${c2}" opacity=".6"/>`;
    case 'moon': return `<circle cx="68" cy="30" r="17" fill="${c2}"/><circle cx="76" cy="24" r="15" fill="currentColor"/>`;
    case 'rain': return [10, 30, 50, 70, 90, 110].map((x, i) => `<path d="M${x} ${i % 2 ? -4 : 6}l-14 36" stroke="${c2}" stroke-width="2.5" stroke-linecap="round"/>`).join('');
    case 'glass': return `<path d="M56 16h30L71 36z" fill="${c2}"/><path d="M71 36v18M63 56h16" stroke="${c2}" stroke-width="3" stroke-linecap="round"/>`;
    case 'gear': return `<circle cx="70" cy="30" r="13" fill="none" stroke="${c2}" stroke-width="7" stroke-dasharray="5 3.2"/><circle cx="70" cy="30" r="5" fill="${c2}"/>`;
    case 'beam': return `<path d="M74 26L100 12v30z" fill="${c2}" opacity=".55"/><path d="M66 24h10l3 50H63z" fill="${c2}"/>`;
    case 'key': return `<circle cx="66" cy="28" r="10" fill="none" stroke="${c2}" stroke-width="4"/><path d="M73 35l16 16M83 45l5-5M87 49l4-4" stroke="${c2}" stroke-width="4" stroke-linecap="round"/>`;
    case 'leaf': return `<path d="M34 82C34 46 62 24 88 16c-4 30-22 64-54 66z" fill="${c2}"/><path d="M38 78L80 26" stroke="currentColor" stroke-width="2"/>`;
    default: return '';
  }
}
function initials(n) { return n.replace(/^Dr\.\s*/, '').split(/\s+/).map(w => w[0]).slice(0, 2).join(''); }
function av(c, size = 40, round = true) {
  const r = round ? '50%' : '22%';
  return `<span class="av" style="width:${size}px;height:${size}px;border-radius:${r}"><svg viewBox="0 0 100 100" style="color:${c.tone}" aria-hidden="true"><rect width="100" height="100" fill="${c.tone}"/>${motif(c.motif, c.tone2)}<text x="${size < 40 ? 50 : 16}" y="${size < 40 ? 64 : 86}" text-anchor="${size < 40 ? 'middle' : 'start'}" font-family="Segoe UI, system-ui, sans-serif" font-weight="600" font-size="${size < 40 ? 40 : 30}" fill="#f2ece4">${initials(c.name)}</text></svg></span>`;
}
function art(c) {
  return `<svg viewBox="0 0 100 75" preserveAspectRatio="xMidYMid slice" style="color:${c.tone}" aria-hidden="true"><rect width="100" height="75" fill="${c.tone}"/><g transform="translate(6 -6)">${motif(c.motif, c.tone2)}</g><text x="10" y="64" font-family="Segoe UI, system-ui, sans-serif" font-weight="600" font-size="24" fill="#f2ece4">${initials(c.name)}</text></svg>`;
}

/* ================= Toast / Undo ================= */
let toastTimer = null;
function toast(text, opts = {}) {
  const t = $('#toast');
  const ms = opts.ms || (opts.undo ? 8000 : 2600);
  t.innerHTML = `<span>${text}</span>${opts.undo ? `<button data-act="undo">Rückgängig</button>` : ''}${opts.undo ? `<i class="timer" style="--t:${ms}ms"></i>` : ''}`;
  t.classList.add('on');
  toast.undo = opts.undo || null;
  const timer = $('.timer', t);
  if (timer) requestAnimationFrame(() => timer.classList.add('run'));
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.classList.remove('on'); toast.undo = null; }, ms);
}

/* ================= Router ================= */
function parse() {
  const parts = location.hash.replace(/^#\/?/, '').split('/').filter(Boolean);
  return { name: parts[0] || 'chats', a: parts[1], b: parts[2] };
}
const SEC = { chats: 'chats', chat: 'chats', charaktere: 'charaktere', charakter: 'charaktere', entdecken: 'entdecken', einstellungen: 'einstellungen' };

function render() {
  const r = parse();
  stopStream(true);
  const app = $('#app');
  app.classList.toggle('bare', r.name === 'erststart');
  const views = { erststart: vWizard, chats: vChats, chat: vChat, charaktere: vChars, charakter: vEditor, entdecken: vDiscover, einstellungen: vSettings };
  const fn = views[r.name] || vChats;
  $('#view').innerHTML = fn(r);
  $$('#nav a, #tabbar a').forEach(a => { if (a.dataset.sec === SEC[r.name]) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current'); });
  after(r);
}
function after(r) {
  if (r.name === 'chat') {
    const st = $('.stream');
    if (st) { st.style.scrollBehavior = 'auto'; st.scrollTop = st.scrollHeight; st.style.scrollBehavior = ''; }
    bindLongPress();
  }
  if (r.name === 'erststart' && (r.a || '1') === '2') runEngineDownload();
  if (r.name !== 'chat') $('#view').scrollTop = 0;
}
function renderChrome() {
  $('#recent').innerHTML = CHATS.slice(0, 3).map(c => { const ch = charById(c.char); return `<a href="#/chat/${c.id}">${av(ch, 22)}<span>${esc(c.title)}</span></a>`; }).join('');
  $('#personaRow').innerHTML = `${av(PERSONA, 26)}<div>Persona <b>${PERSONA.name}</b></div>`;
  $('#themeBtn').innerHTML = ic(S.theme === 'dark' ? 'sun' : 'moon');
  document.documentElement.style.setProperty('--read-size', (S.set.fontsize || 18) + 'px');
}

/* ================= Ansicht: Chats ================= */
function vChats() {
  const last = CHATS[0], lc = charById(last.char);
  const tags = ['Alle', 'Abenteuer', 'Fantasy', 'Krimi', 'Sci-Fi', 'Gemütlich'];
  const list = filteredChats();
  return `<div class="page">
    <div class="title-row"><h1>Chats</h1><span class="sub num">${CHATS.length} Chats</span>
      <div class="actions"><a class="btn" href="#/charaktere">${ic('plus', 'sm')}Neuer Chat</a></div></div>
    <section class="resume" aria-label="Weiter, wo ich war">
      ${av(lc, 112, false)}
      <div style="min-width:0">
        <div class="who">Weiter, wo du warst · ${esc(lc.name)}</div>
        <h2>${esc(last.title)}</h2>
        <blockquote>${esc(last.last)}</blockquote>
        <div class="meta num">${fmt(last.msgs)} Nachrichten · ${last.tk} Tokens · ${last.when} · Kontext liegt im Cache, öffnet sofort</div>
      </div>
      <div class="go"><a class="btn primary" href="#/chat/${last.id}">Weiterlesen ${ic('right', 'sm')}</a></div>
    </section>
    <div class="toolbar">
      <label class="search"><span class="sr">Chats durchsuchen</span>${ic('search', 'sm')}<input class="input" data-in="chatq" value="${esc(S.chatQuery)}" placeholder="In allen Chats suchen"></label>
      <div class="chips" role="group" aria-label="Tags">${tags.map(t => `<button class="chip" data-act="chattag" data-arg="${t}" aria-pressed="${S.chatTag === t}">${t}</button>`).join('')}</div>
      <span class="right num">${list.length} von ${CHATS.length}</span>
    </div>
    <div id="chatListWrap">${chatListHtml(list)}</div>
  </div>`;
}
function filteredChats() {
  const q = S.chatQuery.toLowerCase();
  return CHATS.filter(c => { const ch = charById(c.char); return (!q || (c.title + ch.name + c.last).toLowerCase().includes(q)) && (S.chatTag === 'Alle' || ch.tags.includes(S.chatTag)); });
}
function chatRow(c) {
  const ch = charById(c.char);
  return `<a class="chat-row" href="#/chat/${c.id}">${av(ch, 44)}<div style="min-width:0"><div class="l1"><b>${esc(ch.name)}</b><span>${esc(c.title)}</span>${c.pinned ? ic('pin', 'pin-ic') : ''}</div><div class="l2">${esc(c.last)}</div></div><div class="r num">${c.when}<small>${fmt(c.msgs)} Nachr. · ${c.tk}</small></div><button class="icon-btn kebab" data-act="toast" data-arg="Menü: Umbenennen · Anheften · Exportieren · Löschen" aria-label="Mehr">${ic('more')}</button></a>`;
}
function chatListHtml(list) {
  const pinned = list.filter(c => c.pinned), rest = list.filter(c => !c.pinned), row = chatRow;
  if (!pinned.length && !rest.length) return `<div class="empty-line">Kein Chat passt zu „${esc(S.chatQuery)}“. Die Suche durchsucht Titel, Namen und Nachrichten.</div>`;
  return `${pinned.length ? `<h2 class="sec">Angeheftet <span class="faint num">${pinned.length}</span></h2><div class="chat-list">${pinned.map(row).join('')}</div>` : ''}
    ${rest.length ? `<h2 class="sec">Alle Chats <span class="faint num">${rest.length}</span></h2><div class="chat-list">${rest.map(row).join('')}</div>` : ''}`;
}

/* ================= Ansicht: Chat ================= */
function msgText(m) { return m.variants ? m.variants[m.sw] : m.text; }
function msgHtml(m, i, arr) {
  const ch = charById('maren');
  const isLast = i === arr.length - 1 && m.who === 'char';
  const who = m.who === 'char' ? ch : PERSONA;
  const editing = S.editing === m.id;
  const body = editing
    ? `<div class="edit-box"><textarea class="textarea" data-in="edit" aria-label="Nachricht bearbeiten">${esc(msgText(m))}</textarea><div class="row"><button class="btn sm ghost" data-act="edit-cancel">Abbrechen</button><button class="btn sm primary" data-act="edit-save" data-arg="${m.id}">Speichern</button></div></div>`
    : `<div class="prose">${md(msgText(m))}</div>`;
  const swipe = m.who === 'char' ? `<span class="swipe"><button class="icon-btn" data-act="swipe" data-arg="${m.id}|-1" aria-label="Vorherige Variante">${ic('left', 'sm')}</button><span class="num">${m.sw + 1}/${m.swN}</span><button class="icon-btn" data-act="swipe" data-arg="${m.id}|1" aria-label="Nächste Variante">${ic('right', 'sm')}</button></span>` : '';
  const tools = [
    ['edit', 'Bearbeiten', 'm-edit'], ['copy', 'Kopieren', 'm-copy'],
    ...(m.who === 'char' ? [['redo', 'Neu generieren', 'm-regen'], ['continue', 'Weiter schreiben', 'm-cont']] : []),
    ['branch', 'Branch ab hier', 'm-branch'], ['hide', m.hidden ? 'Wieder in den Kontext' : 'Aus Kontext ausblenden', 'm-hide'],
    ['trash', 'Löschen', 'm-del'], ['info', 'Info', 'm-info'],
  ].map(([i2, l, a]) => `<button class="icon-btn" data-act="${a}" data-arg="${m.id}" title="${l}" aria-label="${l}">${ic(i2, 'sm')}</button>`).join('');
  const info = S.info === m.id ? `<dl class="info-pop num"><div><dt>Tokens</dt><dd>${m.tokens}</dd></div>${m.tps ? `<div><dt>Tempo</dt><dd>${String(m.tps).replace('.', ',')} tok/s</dd></div><div><dt>Dauer</dt><dd>${(m.tokens / m.tps).toFixed(1).replace('.', ',')} s</dd></div><div><dt>Modell</dt><dd>Orion 26B · Q4_K_M</dd></div><div><dt>Preset</dt><dd>Ausgewogen</dd></div><div><dt>Cache</dt><dd>97 %</dd></div>` : `<div><dt>Geschrieben</dt><dd>${m.time}</dd></div>`}</dl>` : '';
  const think = m.think ? `<details class="think"><summary>${ic('down', 'sm')}Gedanken · <span class="num">${m.thinkMeta}</span></summary><p>${esc(m.think)}</p></details>` : '';
  const fixes = isLast ? `<div class="fixes"><span class="lbl">Schnell-Fix</span>${['Kürzer', 'Länger', 'Mehr Dialog', 'Weniger blumig', 'Anders'].map(f => `<button class="chip" data-act="fix" data-arg="${f}">${f}</button>`).join('')}</div>` : '';
  return `<article class="msg ${m.who === 'char' ? 'ai' : 'user'} ${isLast ? 'last' : ''} ${m.hidden ? 'hidden-ctx' : ''}" data-mid="${m.id}">
    <header class="msg-head">${av(who, 28)}<b>${m.who === 'char' ? 'Maren' : 'Du (Lukas)'}</b><time class="num">${m.time}</time></header>
    ${think}${body}
    <div class="ctx-note">Ausgeblendet – zählt nicht mehr zum Kontext.</div>
    ${editing ? '' : `<div class="tools">${swipe}<span class="vr"></span>${tools}</div>`}
    ${info}${fixes}
    ${isLast ? '<div class="longpress-hint">Wischen für Varianten · lange drücken auf ältere Nachrichten für Aktionen</div>' : ''}
  </article>`;
}
function vChat(r) {
  const chat = CHATS.find(c => c.id === r.a) || CHATS[0];
  const ch = charById(chat.char);
  const isMaren = ch.id === 'maren';
  const cls = ['chat', S.story ? 'story' : '', S.scene ? '' : 'no-scene', S.sceneOpen ? 'scene-open' : ''].join(' ');
  const msgs = isMaren ? S.msgs.map(msgHtml).join('') : `<article class="msg ai last"><header class="msg-head">${av(ch, 28)}<b>${esc(ch.name.split(' ')[0])}</b><time>gestern</time></header><div class="prose">${md(chat.last)}</div></article>`;
  return `<div class="${cls}" id="chatRoot">
    <div class="chat-main">
      <div class="chat-head">
        ${av(ch, 36)}
        <div class="t"><b>${esc(ch.name)}</b><span>„${esc(chat.title)}“ · <span class="num">${fmt(chat.msgs)}</span> Nachrichten</span></div>
        <div class="seg" role="group" aria-label="Ansicht"><button data-act="mode" data-arg="chat" aria-pressed="${!S.story}">Chat</button><button data-act="mode" data-arg="story" aria-pressed="${S.story}">Story</button></div>
        <button class="icon-btn hide-sm" data-act="toast" data-arg="Branch-Baum: 3 Abzweigungen in diesem Chat" title="Branch-Baum" aria-label="Branch-Baum">${ic('branch')}</button>
        <button class="icon-btn" data-act="scene" aria-pressed="${S.scene}" title="Szene-Leiste" aria-label="Szene-Leiste">${ic('scene')}</button>
      </div>
      <div class="stream" id="stream"><div class="col">
        <div class="day">Heute · Szene begann um 20:41</div>
        ${msgs}
      </div></div>
      ${composerHtml()}
    </div>
    ${sceneHtml()}
  </div>`;
}
function composerHtml() {
  const streaming = !!S.streaming;
  return `<div class="composer"><div class="col">
    ${S.set.suggest !== false ? `<div class="suggest" aria-label="Vorschläge">${SUGGESTIONS.map((s, i) => `<button class="chip" data-act="sugg" data-arg="${i}" title="${esc(s.replace(/\*/g, ''))}">${esc(s.replace(/\*/g, ''))}</button>`).join('')}</div>` : ''}
    ${S.regie ? `<div class="regie">${ic('clapper', 'sm')}<input data-in="regie" value="${esc(S.regieText)}" placeholder="Regie für die nächste Antwort, z. B. „lass sie wütend werden“" aria-label="Regie-Anweisung"><small>unsichtbar · nur 1 Antwort</small><button class="icon-btn" data-act="regie" aria-label="Regie schließen">${ic('close', 'sm')}</button></div>` : ''}
    <div class="box">
      <textarea id="composer" rows="1" placeholder="Schreib deinen Zug …" aria-label="Nachricht"></textarea>
      <button class="icon-btn" data-act="imp" title="Impersonate – schreibt deinen Zug ins Feld" aria-label="Impersonate">${ic('mask')}</button>
      <button class="icon-btn" data-act="regie" aria-pressed="${S.regie}" title="Regie-Anweisung" aria-label="Regie-Anweisung">${ic('clapper')}</button>
      <button class="send" data-act="${streaming ? 'stop' : 'send'}" title="${streaming ? 'Stop' : 'Senden'}" aria-label="${streaming ? 'Stop' : 'Senden'}">${ic(streaming ? 'stop' : 'send')}</button>
    </div>
    <div class="hint"><span>Enter senden · Umschalt+Enter neue Zeile</span><span>Strg+K Befehle</span><span class="num">Antwort ≈ 350 Tokens · Intensität ${S.intMode === 'Aus' ? 'aus' : S.intensity}</span></div>
  </div></div>`;
}
function sceneHtml() {
  const off = S.intMode === 'Aus';
  return `<aside class="scene" aria-label="Szene-Leiste">
    <div class="scene-head"><h2>Szene</h2><button class="icon-btn scene-close" data-act="sceneClose" aria-label="Schließen">${ic('close')}</button></div>
    <section><h3>Persona</h3><button class="persona-pick" data-act="toast" data-arg="Personas: Lukas · Kartograf, Ada · Schmugglerin, + Neue Persona">${av(PERSONA, 30)}<div><b style="font-weight:500">${PERSONA.name}</b><span>${PERSONA.sub}</span></div>${ic('down', 'sm')}</button></section>
    <section>
      <h3>Intensität <small>${off ? 'aus' : `Stufe ${S.intensity} · ${INT_NAMES[S.intensity - 1]}`}</small></h3>
      <div class="seg" role="group" aria-label="Intensitäts-Modus">${['Aus', 'Manuell', 'Auto'].map(m => `<button data-act="intmode" data-arg="${m}" aria-pressed="${S.intMode === m}">${m}</button>`).join('')}</div>
      <div class="meter ${off ? 'off' : ''}" role="group" aria-label="Intensität">${INT_NAMES.map((n, i) => `<button class="${!off && i < S.intensity ? 'on' : ''}" data-act="int" data-arg="${i + 1}" title="Stufe ${i + 1} · ${n}" aria-label="Stufe ${i + 1}"></button>`).join('')}</div>
      <div class="meter-lbl"><span>Ruhig</span><span>Maximal</span></div>
      <p class="d">${S.intMode === 'Auto' ? 'Steigt mit dem Verlauf, höchstens 1 Stufe pro 8 Nachrichten. Du kannst jederzeit runter.' : 'Steuert Erzähltiefe, Tempo und Deutlichkeit der nächsten Antworten.'}</p>
    </section>
    <section><h3>Antwortlänge</h3><div class="seg" role="group" aria-label="Antwortlänge">${['Kurz', 'Mittel', 'Lang'].map(m => `<button data-act="len" data-arg="${m}" aria-pressed="${S.length === m}">${m}</button>`).join('')}</div></section>
    <section><h3>Author's Note <small class="num">64 Tokens · im Schwanz</small></h3><textarea class="textarea note" aria-label="Author's Note">Es stürmt die ganze Nacht. Maren zeigt Gefühle nur in Taten, nie in Worten.</textarea></section>
    <section><h3>Lorebooks <small class="num">${S.lb.filter(Boolean).length} von ${LOREBOOKS.length} an</small></h3>
      ${LOREBOOKS.map((l, i) => `<div class="lb-row">${ic('book', 'sm')}<div><b>${esc(l.name)}</b><span class="num">${l.n} Einträge · ${S.lb[i] ? `${l.act} gerade aktiv` : 'aus'}</span></div><button class="toggle" role="switch" aria-checked="${S.lb[i]}" data-act="lb" data-arg="${i}" aria-label="${esc(l.name)}"></button></div>`).join('')}
    </section>
    <section><h3>Preset</h3><div class="select-wrap"><select class="select" aria-label="Preset"><option>Ausgewogen · Orion RP</option><option>Präzise</option><option>Kreativ</option><option>Wild</option></select>${ic('down', 'sm')}</div></section>
    <section><button class="insp-link" data-act="inspector"><span class="l1">Prompt-Inspektor <span class="num">${fmt(PROMPT.total)} Tokens</span></span><div class="stack" style="margin:0">${stackBars()}</div><span class="faint num" style="font-size:12px">Cache ${PROMPT.cache} % · neu berechnet ${fmt(PROMPT.fresh)} Tokens</span></button></section>
  </aside>`;
}
function stackBars() {
  const t = PROMPT.total;
  return `<i style="width:${PROMPT.headSum / t * 100}%;background:var(--accent)"></i><i style="width:${PROMPT.histSum / t * 100}%;background:var(--accent-dim)"></i><i style="width:${Math.max(PROMPT.tailSum / t * 100, 1.2)}%;background:var(--text)"></i>`;
}

/* Streaming-Simulation */
function stopStream(silent) {
  if (!S.streaming) return;
  clearInterval(S.streaming.timer);
  const m = S.msgs.find(x => x.id === S.streaming.id);
  if (m) { const words = S.streaming.full.split(' '); const part = words.slice(0, S.streaming.i).join(' '); if (m.variants) m.variants[m.sw] = part; else m.text = part; }
  S.streaming = null;
  if (!silent) { rerenderChat(); toast('Gestoppt – der Text bis hier bleibt.'); }
}
function stream(m, full) {
  const words = full.split(' ');
  S.streaming = { id: m.id, full, i: 0, timer: null };
  rerenderChat();
  const el = () => $(`[data-mid="${m.id}"] .prose`);
  S.streaming.timer = setInterval(() => {
    const s = S.streaming; if (!s) return;
    s.i += 2;
    const part = words.slice(0, s.i).join(' ');
    const e = el();
    if (e) { e.innerHTML = md(part); e.lastElementChild && e.lastElementChild.classList.add('cursor'); const st = $('#stream'); if (st) st.scrollTop = st.scrollHeight; }
    if (s.i >= words.length) {
      clearInterval(s.timer);
      if (m.variants) m.variants[m.sw] = full; else m.text = full;
      S.streaming = null; rerenderChat();
    }
  }, 45);
}
function rerenderChat() {
  if (parse().name !== 'chat') return;
  const st = $('#stream'); const atBottom = !st || st.scrollHeight - st.scrollTop - st.clientHeight < 80;
  const prevTop = st ? st.scrollTop : 0;
  const composer = $('#composer'); const draft = composer ? composer.value : '';
  $('#view').innerHTML = vChat(parse());
  const c2 = $('#composer'); if (c2) { c2.value = draft; autosize(c2); }
  const st2 = $('#stream'); if (st2) { st2.style.scrollBehavior = 'auto'; st2.scrollTop = atBottom ? st2.scrollHeight : prevTop; st2.style.scrollBehavior = ''; }
  bindLongPress();
}
function autosize(t) { t.style.height = 'auto'; t.style.height = Math.min(t.scrollHeight, 180) + 'px'; }
function typeInto(t, text) {
  t.value = ''; let i = 0; t.focus();
  const iv = setInterval(() => { i += 3; t.value = text.slice(0, i); autosize(t); if (i >= text.length) clearInterval(iv); }, 16);
}
function bindLongPress() {
  $$('.msg').forEach(el => {
    let timer = null, x0 = 0, y0 = 0;
    el.addEventListener('touchstart', e => {
      x0 = e.touches[0].clientX; y0 = e.touches[0].clientY;
      timer = setTimeout(() => { $$('.msg.show-tools').forEach(o => o !== el && o.classList.remove('show-tools')); el.classList.add('show-tools'); if (navigator.vibrate) navigator.vibrate(12); }, 450);
    }, { passive: true });
    el.addEventListener('touchmove', () => clearTimeout(timer), { passive: true });
    el.addEventListener('touchend', e => {
      clearTimeout(timer);
      const dx = e.changedTouches[0].clientX - x0, dy = e.changedTouches[0].clientY - y0;
      if (el.classList.contains('last') && Math.abs(dx) > 70 && Math.abs(dy) < 40) doSwipe(el.dataset.mid, dx < 0 ? 1 : -1);
    });
  });
}
function doSwipe(id, d) {
  const m = S.msgs.find(x => x.id === id); if (!m || !m.variants) { if (m) toast(m.swN > 1 ? 'Alternative Begrüßung gewechselt' : 'Nur eine Variante – „Neu“ erzeugt weitere'); if (m && m.swN > 1) { m.sw = (m.sw + d + m.swN) % m.swN; } rerenderChat(); return; }
  const next = m.sw + d;
  if (next >= m.variants.length) { m.variants.push(''); m.swN = m.variants.length; m.sw = next; stream(m, LAST_VARIANTS[next % LAST_VARIANTS.length]); return; }
  m.sw = Math.max(0, next); rerenderChat();
}

/* Prompt-Inspektor */
function inspectorHtml() {
  const t = PROMPT.total;
  const rows = arr => arr.map(([n, v]) => `<div class="blk-row ${v === 0 ? 'zero' : ''}"><span>${esc(n)}</span><span class="n">${fmt(v)}</span><span class="p">${(v / t * 100).toFixed(1).replace('.', ',')} %</span><div class="bar thin"><i style="width:${Math.max(v / t * 100, 0)}%"></i></div></div>`).join('');
  return `<div class="drawer-head"><h2>Prompt-Inspektor</h2><button class="icon-btn" data-act="closeAll" aria-label="Schließen">${ic('close')}</button></div>
    <p class="muted" style="margin:4px 0 0">Was beim letzten Zug an das Modell ging – „Sturm vor Kap Morn“.</p>
    <div class="cache-big"><b class="num">${PROMPT.cache} %</b><span class="muted">aus dem Cache</span></div>
    <p class="faint num" style="margin:0;font-size:13px">Neu berechnet: ${fmt(PROMPT.fresh)} von ${fmt(t)} Tokens · Kontext ${fmt(t)} / ${fmt(PROMPT.ctx)}</p>
    <div class="stack">${stackBars()}</div>
    <div class="stack-legend"><span><i style="background:var(--accent)"></i>Kopf ${fmt(PROMPT.headSum)}</span><span><i style="background:var(--accent-dim)"></i>Verlauf ${fmt(PROMPT.histSum)}</span><span><i style="background:var(--text)"></i>Schwanz ${fmt(PROMPT.tailSum)}</span></div>
    <div class="blk"><div class="blk-head"><b>Kopf</b><span>bleibt gleich · einmal eingelesen</span></div>${rows(PROMPT.head)}</div>
    <div class="blk"><div class="blk-head"><b>Verlauf</b><span>wächst nur hinten</span></div>${rows([['138 Nachrichten (4 ausgeblendet)', PROMPT.histSum]])}</div>
    <div class="blk"><div class="blk-head"><b>Schwanz</b><span>wechselt jede Runde · klein halten</span></div>${rows(PROMPT.tail)}</div>
    <div class="well" style="margin-top:22px;font-size:13px;display:grid;gap:4px"><b style="font-weight:600">Kein Cache-Verlust in diesem Zug</b><span class="muted">Zufalls- und Zeit-Makros im Kopf wurden einmal pro Chat festgelegt. Kontext voll bei 128k → dann werden einmal ~25 % des Verlaufs gekürzt.</span></div>
    <div style="display:flex;gap:8px;margin-top:16px;flex-wrap:wrap"><a class="btn sm" href="#/einstellungen/experte" data-act="closeAll">Roh-Prompt (Experte)</a><button class="btn sm ghost" data-act="toast" data-arg="Prompt kopiert">Als Text kopieren</button></div>`;
}

/* ================= Ansicht: Charaktere ================= */
function vChars() {
  const filters = ['Alle', 'Favoriten', 'Fantasy', 'Sci-Fi', 'Abenteuer'];
  return `<div class="page">
    <div class="title-row"><h1>Charaktere</h1><span class="sub num">${CHARS.length} Karten</span>
      <div class="actions"><button class="btn" data-act="toast" data-arg="Personas: Lukas · Ada">Personas</button><button class="btn" data-act="toast" data-arg="Lorebooks: 3 eigene, 1 global">Lorebooks</button><button class="btn primary" data-act="toast" data-arg="Dateiauswahl: PNG, JSON, CHARX">${ic('upload', 'sm')}Importieren</button></div></div>
    <div class="dropzone" id="dropzone">${ic('upload', 'lg')}<div><b>Karten hierher ziehen</b>PNG, JSON oder CHARX – auch viele auf einmal. Jede Karte wird beim Import geprüft.</div>
      <div class="btns"><button class="btn sm" data-act="toast" data-arg="Link-Import: Adresse einfügen">Per Link</button><button class="btn sm" data-act="toast" data-arg="SillyTavern-Ordner wählen …">SillyTavern übernehmen</button></div></div>
    <div class="toolbar">
      <label class="search"><span class="sr">Charaktere suchen</span>${ic('search', 'sm')}<input class="input" data-in="charq" value="${esc(S.charQuery)}" placeholder="Name, Tag, Beschreibung"></label>
      <div class="chips" role="group" aria-label="Filter">${filters.map(t => `<button class="chip" data-act="charf" data-arg="${t}" aria-pressed="${S.charFilter === t}">${t}</button>`).join('')}</div>
      <span class="right">Sortiert nach: zuletzt benutzt</span>
    </div>
    <div class="grid-chars" id="charGrid">${charGridHtml()}</div>
  </div>`;
}
function charGridHtml() {
  const q = S.charQuery.toLowerCase();
  const list = CHARS.filter(c => (!q || (c.name + c.sub + c.tags.join(' ')).toLowerCase().includes(q)) &&
    (S.charFilter === 'Alle' || (S.charFilter === 'Favoriten' && S.favs[c.id]) || c.tags.includes(S.charFilter)));
  if (!list.length) return `<div class="empty-line" style="grid-column:1/-1">Keine Karte passt. Filter „${esc(S.charFilter)}“${q ? ` und „${esc(S.charQuery)}“` : ''}.</div>`;
  return list.map(c => `<a class="cc" href="#/charakter/${c.id}/uebersicht">
    <span class="art">${art(c)}</span>
    <button class="fav" data-act="fav" data-arg="${c.id}" aria-pressed="${!!S.favs[c.id]}" aria-label="Favorit">${ic('star', 'sm')}</button>
    <div class="body"><b>${esc(c.name)}</b><span class="s">${esc(c.sub)}</span>
      <div class="tags">${c.tags.map(t => `<span class="tag">${t}</span>`).join('')}</div>
      <div class="meta num"><span>${fmt(c.tokens)} Tokens</span><span>${c.chats ? `${c.chats} Chat${c.chats > 1 ? 's' : ''}` : 'noch kein Chat'}</span></div></div>
  </a>`).join('');
}

/* ================= Ansicht: Charakter-Editor ================= */
const ED_TABS = [['uebersicht', 'Übersicht'], ['persoenlichkeit', 'Persönlichkeit'], ['dialoge', 'Dialoge'], ['lorebook', 'Lorebook'], ['erweitert', 'Erweitert'], ['check', 'Check']];
const MAREN = {
  desc: 'Maren Holt, 34, Kapitänin des Dreimast-Schoners „Sturmschwalbe“. Wettergegerbtes Gesicht, kurzes, salzverkrustetes dunkelblondes Haar, eine helle Narbe über der linken Augenbraue. Trägt einen abgewetzten Kapitänsrock über Hemd und Weste. Hat das Schiff von ihrem Vater übernommen und führt es seit acht Jahren durch Gewässer, die andere meiden. {{char}} spricht knapp und trocken, oft mit einem Hauch Spott.',
  pers: 'stolz, pflichtbewusst, misstrauisch gegenüber Fremden, heimlich romantisch; lacht selten – dann aber ehrlich; zeigt Zuneigung durch Taten statt Worte',
  scen: 'Herbst 1821. Die Sturmschwalbe liegt vor Kap Morn im Sturm. {{user}}, ein Kartograf im Auftrag der Handelsgesellschaft, kommt mit einer alten Karte an Bord, die eine vergessene Einfahrt zeigt. An Bord ist es {{time}} Uhr.',
  dlg: '<START>\n{{user}}: Haben Sie keine Angst vor dem Sturm?\n{{char}}: *Sie zieht die Handschuhe fester.* „Angst habe ich vor Leuten, die keine haben. Halten Sie sich fest.“\n<START>\n{{user}}: Warum fahren Sie allein?\n{{char}}: „Allein? Ich habe vierzehn Mann an Bord.“ *Ein Blick zur Seite.* „Aber Sie meinen etwas anderes. Fragen Sie das nicht noch einmal.“',
  greets: [INITIAL_MSGS()[0].text, '*Der Hafen von Velde riecht nach Teer und Regen. Maren lehnt an einem Poller und zählt Fässer, als du den Kai entlangkommst.* „Sie schon wieder. Haben Sie eine neue Karte, oder nur eine neue Ausrede?“', '„Wir legen in einer Stunde ab.“ *Sie reicht dir ein Fernrohr.* „Wer an Bord will, trägt etwas. Sie tragen das hier.“'],
  post: 'Bleib in Marens Perspektive. Schreib nie Handlungen oder Worte für {{user}}.',
  notes: 'Slow Burn – Maren öffnet sich nur, wenn {{user}} Mut zeigt. Getestet mit Orion 26B und Qwen3.8.',
};
function fieldsFor(c) {
  if (c.id === 'maren') return MAREN;
  return {
    desc: `${c.name}${c.age ? `, ${c.age}` : ''}, ${c.sub}. ${c.id === 'moos' ? 'Ein kleiner, moosbewachsener Geist mit Laternenaugen, der Wanderern den Weg über den Grenzpfad zeigt.' : 'Eigene Beispielfigur für den Prototyp.'}`,
    pers: c.id === 'moos' ? 'neugierig, freundlich, scheu bei Lärm, liebt Rätsel und Pilze' : 'eigenständig, schlagfertig, mit eigenen Zielen',
    scen: `{{user}} begegnet ${c.name.split(' ')[0]} zum ersten Mal.`, dlg: '<START>\n{{user}}: Hallo.\n{{char}}: *Ein prüfender Blick.* „Hallo zurück.“',
    greets: [`*${c.name.split(' ')[0]} sieht auf.* „Ah. Du bist also gekommen.“`], post: '', notes: '',
  };
}
function vEditor(r) {
  const c = charById(r.a); const tab = r.b || 'uebersicht'; const f = fieldsFor(c);
  const head = `<a class="back" href="#/charaktere">${ic('left', 'sm')}Charaktere</a>
    <div class="obj-head">${av(c, 72, false)}<div style="min-width:0"><h1>${esc(c.name)}</h1><div class="meta">${esc(c.sub)} · <span class="num">${fmt(c.tokens)}</span> Tokens</div>
      <div class="tags-row">${c.tags.map(t => `<span class="tag">${t}</span>`).join('')}</div></div>
      <div class="actions"><button class="btn" data-act="toast" data-arg="Export: PNG (V2) · JSON · CHARX">${ic('download', 'sm')}Exportieren</button><a class="btn primary" href="#/chat/${(CHATS.find(x => x.char === c.id) || CHATS[0]).id}">Chat starten</a></div></div>
    <nav class="tabs" aria-label="Bereiche">${ED_TABS.map(([id, l]) => `<a href="#/charakter/${c.id}/${id}" ${id === tab ? 'aria-current="page"' : ''}>${l}${id === 'check' ? ' <span class="faint num">2</span>' : ''}</a>`).join('')}</nav>`;
  const fld = (label, val, rows = 5, key = '') => `<div class="field"><label>${label}<small data-count>${fmt(tok(val))} Tokens</small></label><textarea class="textarea" rows="${rows}" data-in="count" ${key ? `data-key="${key}"` : ''}>${esc(val)}</textarea></div>`;
  let body = '';
  if (tab === 'uebersicht') {
    const g = Math.min(S.greet, f.greets.length - 1);
    body = `<div class="ed-grid"><div>
      <div class="field"><label>Name</label><input class="input" value="${esc(c.name)}"></div>
      <div class="field"><label>Untertitel</label><input class="input" value="${esc(c.sub)}"></div>
      <div class="field"><label>Tags</label><input class="input" value="${esc(c.tags.join(', '))}"></div>
      ${fld(`Begrüßung ${g + 1} von ${f.greets.length}`, f.greets[g], 7)}
    </div><div class="aside-card"><h3>Vorschau der Begrüßung</h3>
      <div class="greet-nav"><button class="icon-btn" data-act="greet" data-arg="-1" aria-label="Vorherige">${ic('left', 'sm')}</button><span class="num">${g + 1} / ${f.greets.length}</span><button class="icon-btn" data-act="greet" data-arg="1" aria-label="Nächste">${ic('right', 'sm')}</button><span style="margin-left:auto">im Chat per Wischen</span></div>
      <div class="greet">${md(f.greets[g])}</div></div></div>`;
  } else if (tab === 'persoenlichkeit') {
    body = `<div class="ed-grid"><div>${fld('Beschreibung', f.desc, 7)}${fld('Persönlichkeit', f.pers, 3)}${fld('Szenario', f.scen, 4)}</div>
      <div class="aside-card"><h3>Im Prompt</h3><p class="muted" style="margin:0 0 12px;font-size:13px">Diese drei Felder stehen im Kopf – sie werden einmal eingelesen und bleiben im Cache.</p>
      <div class="well" style="font-size:13px">Makros: <code>{{char}}</code> <code>{{user}}</code> <code>{{time}}</code><br><span class="muted">{{time}} wird einmal pro Chat festgelegt, sonst bremst es den Cache.</span></div></div></div>`;
  } else if (tab === 'dialoge') {
    body = `<div class="ed-grid"><div>${fld('Beispieldialoge', f.dlg, 12)}</div><div class="aside-card"><h3>Verhalten</h3>
      <div class="seg" role="group"><button aria-pressed="true">Immer</button><button>Bis Kontext voll</button><button>Nie</button></div>
      <p class="muted" style="font-size:13px;margin:12px 0 0">Beispieldialoge zeigen dem Modell den Ton. Jeder Block beginnt mit <code>&lt;START&gt;</code>.</p></div></div>`;
  } else if (tab === 'lorebook') {
    body = loreHtml();
  } else if (tab === 'erweitert') {
    body = `<div class="ed-grid"><div>
      ${fld('Eigener Systemprompt', '', 3)}
      ${fld('Post-History-Anweisung', f.post, 3)}
      <div class="field"><label>Intensitäts-Tempo <small>für Auto-Modus</small></label><div class="seg" role="group"><button>Langsam</button><button aria-pressed="true">Mittel</button><button>Schnell</button></div><p class="muted" style="font-size:13px;margin:8px 0 0">Mittel = höchstens 1 Stufe pro 8 Nachrichten.</p></div>
      ${fld('Creator-Notes', f.notes, 3)}
    </div><div class="aside-card"><h3>Hinweis</h3><p class="muted" style="margin:0;font-size:13px">Leer = der Standard-Systemprompt der App.</p></div></div>`;
  } else {
    body = checkHtml(c, f);
  }
  return `<div class="page">${head}${body}</div>`;
}
function loreHtml() {
  const t = S.loreTest.toLowerCase();
  const hit = e => e.always || e.keys.some(k => t.includes(k.toLowerCase()));
  const rows = LORE_ENTRIES.map(e => `<tr class="${hit(e) ? 'lore-hit' : ''}"><td>${e.keys.map(esc).join(', ')}</td><td class="muted">${esc(e.text)}</td><td>${e.pos}</td><td class="num">${e.tk}</td></tr>`).join('');
  const n = LORE_ENTRIES.filter(hit).length, sumTk = LORE_ENTRIES.filter(hit).reduce((s, e) => s + e.tk, 0);
  return `<div class="tester well"><label style="display:block;font-weight:500;margin-bottom:8px">Tester – Text eingeben, sehen welche Einträge feuern</label>
      <input class="input" data-in="loretest" value="${esc(S.loreTest)}" aria-label="Testtext"><p class="muted num" id="loreSum" style="margin:8px 0 0;font-size:13px">${n} von ${LORE_ENTRIES.length} Einträgen feuern · ${sumTk} Tokens · Budget 15 % des Kontexts</p></div>
    <div style="overflow-x:auto;margin-top:18px"><table class="data" id="loreTable"><thead><tr><th>Schlüssel</th><th>Inhalt</th><th>Position</th><th class="num">Tokens</th></tr></thead><tbody>${rows}</tbody></table></div>
    <button class="btn sm" style="margin-top:14px" data-act="toast" data-arg="Neuer Eintrag angelegt">${ic('plus', 'sm')}Eintrag</button>`;
}
function checkHtml(c, f) {
  const fields = c.id === 'maren'
    ? [['Beschreibung', 'Kopf', tok(f.desc) * 4], ['Persönlichkeit', 'Kopf', 186], ['Szenario', 'Kopf', 228], ['Beispieldialoge', 'Kopf', 960], ['Lorebook (5 Einträge)', 'Schwanz', 350], ['Post-History', 'Schwanz', 120], ['Begrüßungen (3)', 'Verlauf', 572]]
    : [['Beschreibung', 'Kopf', tok(f.desc)], ['Persönlichkeit', 'Kopf', tok(f.pers)], ['Szenario', 'Kopf', tok(f.scen)], ['Beispieldialoge', 'Kopf', tok(f.dlg)], ['Begrüßungen (1)', 'Verlauf', tok(f.greets[0])]];
  if (c.id === 'maren') fields[0][2] = 642;
  else { // Beispielfelder auf die Kartengröße hochrechnen, damit Kopfzeile und Check dieselbe Summe zeigen
    const raw = fields.reduce((s, r) => s + r[2], 0);
    fields.forEach(r => { r[2] = Math.round(r[2] * c.tokens / raw); });
    fields[0][2] += c.tokens - fields.reduce((s, r) => s + r[2], 0);
  }
  const total = fields.reduce((s, r) => s + r[2], 0);
  console.assert(total === c.tokens, 'Check-Summe muss zur Karte passen');
  const max = Math.max(...fields.map(r => r[2]));
  const warns = [['w', 'Makro {{time}} im Kopf bremst den Cache', 'Steht im Szenario. Ein Zeit-Makro im Kopf ändert sich jede Runde – dann muss alles neu eingelesen werden.', 'Einmal pro Chat festlegen'],
       ['h', 'Beispieldialoge sind 960 Tokens lang', 'Fast so lang wie Beschreibung, Persönlichkeit und Szenario zusammen. Bei 32k Kontext lohnt sich Kürzen.'],
       ['ok', 'Keine kaputten Makros, kein HTML, keine externen Bilder', 'Alle 4 Makros sind bekannt.']];
  return `<div class="ed-grid check-grid"><div>
    <h2 class="sec" style="margin-top:0">Tokens je Feld <span class="faint num">${fmt(total)} gesamt</span></h2>
    <table class="data"><thead><tr><th>Feld</th><th>Im Prompt</th><th style="width:38%"></th><th class="num">Tokens</th></tr></thead><tbody>
      ${fields.map(([n, p, v]) => `<tr><td>${n}</td><td class="muted">${p}</td><td><div class="bar thin"><i style="width:${v / max * 100}%"></i></div></td><td class="num">${fmt(v)}</td></tr>`).join('')}
    </tbody></table>
    <h2 class="sec">Hinweise</h2>
    <ul class="warn-list">${warns.map(([k, t, d, a]) => `<li class="${k}">${ic(k === 'ok' ? 'check' : k === 'w' ? 'warn' : 'info', 'sm')}<div><b>${esc(t)}</b><span>${esc(d)}</span>${a ? `<div style="margin-top:10px"><button class="btn sm" data-act="toast" data-arg="Erledigt: {{time}} wird einmal pro Chat festgelegt">${a}</button></div>` : ''}</div></li>`).join('')}</ul>
  </div></div>`;
}

/* ================= Ansicht: Entdecken ================= */
function vDiscover(r) {
  const tab = r.a || 'modelle';
  const tabs = [['modelle', 'Modelle'], ['verbindungen', 'Verbindungen'], ['presets', 'Presets'], ['import', 'Import']];
  const nav = `<nav class="tabs" aria-label="Entdecken">${tabs.map(([id, l]) => `<a href="#/entdecken/${id}" ${id === tab ? 'aria-current="page"' : ''}>${l}</a>`).join('')}</nav>`;
  let body = '';
  if (tab === 'modelle') body = modelsHtml();
  else if (tab === 'verbindungen') body = `<div class="tbl-wrap"><table class="data"><thead><tr><th>Verbindung</th><th>Art</th><th>Status</th><th></th></tr></thead><tbody>
      <tr><td><b>Eigene Engine</b> <span class="muted">· llama.cpp ROCm</span></td><td class="muted">lokal</td><td><span class="tag badge-ok">Läuft</span></td><td></td></tr>
      <tr><td>LM Studio</td><td class="muted">OpenAI-kompatibel · localhost:1234</td><td><span class="tag">Nicht verbunden</span></td><td style="text-align:right"><button class="btn sm" data-act="toast" data-arg="Verbindung wird geprüft …">Verbinden</button></td></tr>
      <tr><td>KoboldCpp</td><td class="muted">Text-Server · localhost:5001</td><td><span class="tag">Nicht verbunden</span></td><td style="text-align:right"><button class="btn sm" data-act="toast" data-arg="Verbindung wird geprüft …">Verbinden</button></td></tr>
    </tbody></table></div><button class="btn sm" style="margin-top:14px" data-act="toast" data-arg="Ollama · OpenRouter · eigene Adresse">${ic('plus', 'sm')}Verbindung</button>`;
  else if (tab === 'presets') body = `<div class="tbl-wrap"><table class="data"><thead><tr><th>Preset</th><th>Wirkung</th><th class="num">Kreativität</th><th class="num">Min-P</th></tr></thead><tbody>
      ${[['Präzise', 'eng an der Szene, wenig Abschweifen', '0,70', '0,08'], ['Ausgewogen · Orion RP', 'Standard für lange Rollenspiele', '0,90', '0,05'], ['Kreativ', 'mehr Überraschungen, mehr Bilder', '1,10', '0,04'], ['Wild', 'experimentell, verliert öfter den Faden', '1,35', '0,03']].map((p, i) => `<tr class="${i === 1 ? 'sel' : ''}"><td>${p[0]}</td><td class="muted">${p[1]}</td><td class="num">${p[2]}</td><td class="num">${p[3]}</td></tr>`).join('')}
    </tbody></table></div>`;
  else body = `<div class="dropzone">${ic('upload', 'lg')}<div><b>Datei, Link oder SillyTavern-Ordner</b>Karten (PNG/JSON/CHARX), Lorebooks, Presets und ganze Chats.</div><div class="btns"><button class="btn sm" data-act="toast" data-arg="Dateiauswahl">Datei</button><button class="btn sm" data-act="toast" data-arg="Link einfügen">Link</button><button class="btn sm" data-act="toast" data-arg="SillyTavern-Ordner wählen …">SillyTavern-Ordner</button></div></div>`;
  return `<div class="page"><div class="title-row"><h1>Entdecken</h1><span class="sub">Modelle, Verbindungen, Presets und Import</span></div>${nav}${body}</div>`;
}
function modelsHtml() {
  const L = MODELS.loading;
  const used = 21.4, other = 2.1, total = 24;
  const sel = MODELS.installed.find(m => m.id === S.modelSel) || MODELS.installed[0];
  const done = (L.gb * S.loadPct / 100);
  return `<div class="vram"><div><div class="faint" style="font-size:12px">Grafikspeicher</div><b class="num">${String(used).replace('.', ',')} <span class="muted" style="font-size:14px;font-weight:400">/ ${total} GB</span></b></div>
      <div><div class="stack">${`<i style="width:${(used - other) / total * 100}%;background:var(--accent)"></i><i style="width:${other / total * 100}%;background:var(--muted)"></i>`}</div>
      <div class="stack-legend" style="margin-top:8px"><span><i style="background:var(--accent)"></i>Orion + 128k Kontext ${String((used - other).toFixed(1)).replace('.', ',')} GB</span><span><i style="background:var(--muted)"></i>ComfyUI ${String(other).replace('.', ',')} GB</span><span><i style="background:var(--raised)"></i>frei ${String((total - used).toFixed(1)).replace('.', ',')} GB</span></div></div>
      <button class="btn sm" data-act="toast" data-arg="ComfyUI hat 2,1 GB freigegeben">Andere freigeben</button></div>
    <div class="models"><div>
      <h2 class="sec">Installiert <span class="faint num">${MODELS.installed.length}</span></h2>
      ${MODELS.installed.map(m => `<div class="m-row ${m.id === S.modelSel ? 'sel' : ''}" role="button" tabindex="0" data-act="msel" data-arg="${m.id}"><div><b>${m.name}</b> ${m.active ? '<span class="tag badge-ok">Aktiv</span>' : ''}<div class="s num">${m.q} · ${String(m.gb).replace('.', ',')} GB · ${m.note} · ${m.where}</div></div>${m.active ? '' : `<button class="btn sm" data-act="toast" data-arg="Modell wird geladen …">Laden</button>`}</div>`).join('')}
      <h2 class="sec">Wird geladen</h2>
      <div class="m-row"><div><b>${L.name}</b><div class="s num">${L.q} · ${String(L.gb).replace('.', ',')} GB · fortsetzbar, Prüfsumme danach</div></div><button class="btn sm" data-act="dlpause">${ic(S.loadPaused ? 'download' : 'pause', 'sm')}${S.loadPaused ? 'Weiter' : 'Pause'}</button>
        <div class="prog"><div class="bar"><i style="width:${S.loadPct}%"></i></div><small class="num"><span>${S.loadPct} % · ${done.toFixed(1).replace('.', ',')} / ${String(L.gb).replace('.', ',')} GB</span><span>${S.loadPaused ? 'pausiert' : `${L.speed} MB/s · noch ${Math.ceil((L.gb - done) * 1024 / L.speed / 60)} Min`}</span></small></div></div>
      <h2 class="sec">Empfohlen für 24 GB <span class="faint">frei zugänglich</span></h2>
      ${MODELS.recommended.map(m => `<div class="m-row"><div><b>${m.name}</b><div class="s num">${m.q} · ${String(m.gb).replace('.', ',')} GB · ${m.note}</div></div><button class="btn sm" data-act="toast" data-arg="Download gestartet: ${m.name}">${ic('download', 'sm')}Laden</button></div>`).join('')}
    </div>
    <div class="card prof">
      <h2>${sel.name}</h2><p class="muted" style="margin:4px 0 0">Modellprofil · aus der GGUF gelesen</p>
      <dl class="kv num">
        <div><dt>Größe</dt><dd>${sel.id === 'orion' ? '26B (4B aktiv)' : '14B'}</dd></div><div><dt>Quantisierung</dt><dd>${sel.q}</dd></div><div><dt>Datei</dt><dd>${String(sel.gb).replace('.', ',')} GB</dd></div>
        <div><dt>Max. Kontext</dt><dd>128k</dd></div><div><dt>Template</dt><dd>${sel.id === 'orion' ? 'gemma4 (aus Datei)' : 'chatml (aus Datei)'}</dd></div><div><dt>Besonderheit</dt><dd>${sel.id === 'orion' ? 'SWA, Checkpoints' : '—'}</dd></div>
      </dl>
      <h2 class="sec" style="margin-top:22px">Auto-Tune ${sel.id === 'orion' ? '<span class="faint num">gemessen 12.11.2026 · 1 Min 42 s</span>' : ''}</h2>
      ${sel.id === 'orion' ? `<div class="tune-wrap"><table class="data tune"><thead><tr><th>Rechenweg</th><th>KV-Cache</th><th class="num">tok/s leer</th><th class="num">tok/s 64k</th><th class="num">Einlesen</th><th class="num">Kontext</th><th class="num">VRAM</th></tr></thead><tbody>
        ${TUNE.map(t => `<tr class="${t.best ? 'sel' : ''}"><td>${t.best ? '<span class="mark"></span>' : ''}${t.be}</td><td>${t.kv}</td><td class="num">${t.g0}</td><td class="num">${t.g64}</td><td class="num">${fmt(t.pp)}</td><td class="num">${t.ctx}</td><td class="num">${String(t.vram).replace('.', ',')} GB</td></tr>`).join('')}
      </tbody></table></div>
      <p class="muted" style="font-size:13px;margin:12px 0 0">Gewählt: ROCm mit KV q8_0 – fast so schnell wie f16, aber nur so passen 128k Kontext auf 24 GB. ROCm ist hier 58 % schneller als Vulkan.</p>`
      : `<p class="empty-line" style="margin:0">Noch nicht gemessen. Auto-Tune dauert etwa 1–2 Minuten und läuft beim ersten Laden.</p>`}
      <div style="display:flex;gap:8px;margin-top:16px"><button class="btn sm" data-act="toast" data-arg="Auto-Tune startet – etwa 1–2 Minuten">${ic('redo', 'sm')}Erneut messen</button></div>
    </div></div>`;
}

/* ================= Ansicht: Einstellungen ================= */
function ctlHtml(r) {
  const v = S.set[r.key];
  switch (r.type) {
    case 'seg': return `<div class="seg" role="group" aria-label="${esc(r.name)}">${r.opts.map(o => `<button data-act="set" data-key="${r.key}" data-val="${esc(o)}" aria-pressed="${v === o}">${o}</button>`).join('')}</div>`;
    case 'select': return `<div class="select-wrap"><select class="select" data-key="${r.key}" aria-label="${esc(r.name)}">${r.opts.map(o => `<option ${v === o ? 'selected' : ''}>${o}</option>`).join('')}</select>${ic('down', 'sm')}</div>`;
    case 'toggle': return `<button class="toggle" role="switch" aria-checked="${!!v}" data-act="settoggle" data-key="${r.key}" aria-label="${esc(r.name)}"></button>`;
    case 'range': return `<div class="range"><input type="range" min="${r.min}" max="${r.max}" step="${r.step}" value="${v}" data-key="${r.key}" aria-label="${esc(r.name)}"><output class="num">${String(v).replace('.', ',')}</output></div>`;
    case 'text': return `<input class="input" value="${esc(v)}" data-key="${r.key}" aria-label="${esc(r.name)}" ${r.key === 'flags' ? 'style="font-family:var(--mono);font-size:13px"' : ''}>`;
    case 'textarea': return `<textarea class="textarea" data-key="${r.key}" aria-label="${esc(r.name)}">${esc(v)}</textarea>`;
    case 'code': return `<textarea class="textarea code" spellcheck="false" data-key="${r.key}" aria-label="${esc(r.name)}" rows="4">${esc(v)}</textarea>`;
    case 'button': return `<button class="btn sm" data-act="toast" data-arg="${esc(r.label)} – erledigt">${esc(r.label)}</button>`;
    case 'link': return `<a class="btn sm" href="${r.href}">${esc(r.label)}</a>`;
    case 'info': return `<span class="tag badge-ok">${esc(r.value)}</span>`;
    case 'fixed': return `<span class="tag">Nicht abschaltbar</span>`;
    default: return '';
  }
}
function srowHtml(r, showLvl) {
  const changed = r.def !== undefined && r.def !== null && S.set[r.key] !== r.def;
  const canReset = r.def !== undefined && r.def !== null;
  const wide = r.wide ? 'wide' : '';
  const tg = r.type === 'toggle' || r.type === 'fixed' || r.type === 'info' ? 'toggle-row' : '';
  return `<div class="srow ${wide} ${tg} ${changed ? 'changed' : ''}" data-row="${r.key}">
    <div><div class="nm">${esc(r.name)}${r.term ? `<small>(${esc(r.term)})</small>` : ''}${showLvl && r.lvl > 1 ? `<span class="lvl-tag">${LVL[r.lvl]}</span>` : ''}</div><div class="ex">${esc(r.ex)}</div></div>
    <div class="ctl">${ctlHtml(r)}</div>
    ${canReset ? `<button class="icon-btn reset" data-act="reset" data-key="${r.key}" title="Auf Standard zurücksetzen" aria-label="${esc(r.name)} zurücksetzen">${ic('reset', 'sm')}</button>` : '<span></span>'}
  </div>`;
}
function vSettings(r) {
  const pageId = r.a || 'allgemein';
  const page = SETTINGS.find(p => p.id === pageId) || SETTINGS[0];
  const lvlSeg = `<div class="seg" role="group" aria-label="Stufe">${[1, 2, 3].map(l => `<button data-act="lvl" data-arg="${l}" aria-pressed="${S.level === l}">${LVL[l]}</button>`).join('')}</div>`;
  const lvlD = { 1: 'Das Nötigste – reicht für die meisten.', 2: 'Alle Regler, jeder mit Erklärung.', 3: 'Freie Flags, Roh-Prompt, Logs.' }[S.level];
  return `<div class="page">
    <div class="title-row"><h1>Einstellungen</h1></div>
    <div class="set-top">${lvlSeg}<span class="lvl-d">${lvlD}</span>
      <label class="search" style="margin-left:auto"><span class="sr">Einstellungen durchsuchen</span>${ic('search', 'sm')}<input class="input" data-in="setq" value="${esc(S.setQuery)}" placeholder="Einstellung suchen, z. B. Kreativität"></label></div>
    <div class="set-wrap" style="margin-top:18px">
      <nav class="set-nav" aria-label="Bereiche">${SETTINGS.filter(p => p.id !== 'experte' || S.level === 3).map(p => `<a href="#/einstellungen/${p.id}" ${p.id === page.id && !S.setQuery ? 'aria-current="page"' : ''}>${p.title}</a>`).join('')}</nav>
      <div id="setBody">${setBodyHtml(page)}</div>
    </div></div>`;
}
function setBodyHtml(page) {
  const q = S.setQuery.trim().toLowerCase();
  if (q) {
    const hits = ALL_ROWS.filter(r => (r.name + ' ' + (r.term || '') + ' ' + r.ex).toLowerCase().includes(q));
    if (!hits.length) return `<h2 class="sec" style="margin-top:0">Keine Treffer</h2><p class="empty-line">Nichts gefunden für „${esc(S.setQuery)}“. Gesucht wird in Namen, Fachbegriffen und Erklärungen – auf allen Stufen.</p>`;
    const pages = [...new Set(hits.map(h => h.page))];
    return `<h2 class="sec" style="margin-top:0"><span class="num">${hits.length}</span> Treffer für „${esc(S.setQuery)}“</h2>` + pages.map(pid => { const p = SETTINGS.find(x => x.id === pid); return `<p class="search-hit-page">${p.title}</p>${hits.filter(h => h.page === pid).map(h => srowHtml(h, true)).join('')}`; }).join('');
  }
  if (page.custom === 'handy') return handyHtml();
  const groups = page.groups.map(g => ({ ...g, rows: g.rows.filter(r => r.lvl <= S.level) })).filter(g => g.rows.length);
  const hidden = page.groups.reduce((s, g) => s + g.rows.filter(r => r.lvl > S.level).length, 0);
  const body = groups.map(g => `<section class="set-group"><h2 ${g === groups[0] ? 'style="margin-top:0"' : ''}>${g.title}</h2>${g.desc ? `<p class="gd">${g.desc}</p>` : ''}${g.rows.map(r => srowHtml(r, false)).join('')}</section>`).join('');
  return (body || `<h2 class="sec" style="margin-top:0">${page.title}</h2><p class="empty-line">Auf der Stufe „${LVL[S.level]}“ gibt es hier nichts einzustellen.</p>`) +
    (hidden ? `<p class="faint" style="font-size:13px;margin-top:20px"><span class="num">${hidden}</span> weitere Einstellung${hidden > 1 ? 'en' : ''} auf höheren Stufen. <button class="btn sm ghost" data-act="lvl" data-arg="${Math.min(3, S.level + 1)}">${LVL[Math.min(3, S.level + 1)]} zeigen</button></p>` : '');
}
function qrSvg() {
  const n = 25; let seed = 7; const rnd = () => (seed = (seed * 9301 + 49297) % 233280) / 233280;
  let cells = '';
  const finder = (x, y) => { cells += `<rect x="${x}" y="${y}" width="7" height="7" fill="#14110f"/><rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fffcf7"/><rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#14110f"/>`; };
  const inFinder = (x, y) => (x < 8 && y < 8) || (x > n - 9 && y < 8) || (x < 8 && y > n - 9);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) if (!inFinder(x, y) && rnd() > 0.52) cells += `<rect x="${x}" y="${y}" width="1" height="1" fill="#14110f"/>`;
  finder(0, 0); finder(n - 7, 0); finder(0, n - 7);
  return `<svg viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges" role="img" aria-label="QR-Code zum Koppeln (Platzhalter)">${cells}</svg>`;
}
function handyHtml() {
  const wifi = S.set.wifi !== false;
  return `<section class="set-group"><h2 style="margin-top:0">Handy & WLAN</h2><p class="gd">Am Handy läuft dieselbe Oberfläche im Browser. Der PC rechnet, das Handy zeigt an.</p>
    <div class="srow toggle-row"><div><div class="nm">Im WLAN erreichbar</div><div class="ex">${wifi ? 'Erreichbar unter <span class="num">http://192.168.1.20:7860</span> – aber nur für gekoppelte Geräte.' : 'Aus: Die App hört nur auf diesem PC (127.0.0.1).'}</div></div><div class="ctl"><button class="toggle" role="switch" aria-checked="${wifi}" data-act="settoggle" data-key="wifi" aria-label="Im WLAN erreichbar"></button></div><span></span></div>
    <div class="pair">
      <div>
        <h2 style="font-size:15px;font-weight:600;margin:6px 0 8px">Gerät koppeln</h2>
        <ol class="steps-ol"><li>Handy ins selbe WLAN wie diesen PC.</li><li>QR-Code mit der Kamera scannen.</li><li>Auf dem Handy denselben Code bestätigen.</li></ol>
        <h2 style="font-size:15px;font-weight:600;margin:26px 0 4px">Wie soll das Handy die App öffnen?</h2>
        ${[['A', 'Im Browser', 'Geht sofort, nichts zu installieren. Kein eigenes App-Symbol, kein Vollbild.', true, ''],
           ['B', 'Als installierte App', 'Einmal ein Zertifikat per QR installieren (~1 Min). Danach Vollbild und App-Symbol.', false, 'Einrichten'],
           ['C', 'Android-Hülle', 'Kleine APK von GitHub, ohne Zertifikat. Nicht im Play Store.', false, 'APK-Seite']]
          .map(([k, t, d, on, b]) => `<label class="stage"><input type="radio" name="stage" ${on ? 'checked' : ''}><div><b>Stufe ${k} · ${t}</b><p>${d}</p></div>${b ? `<button class="btn sm" data-act="toast" data-arg="${b} …">${b}</button>` : '<span class="tag badge-ok">Aktiv</span>'}</label>`).join('')}
      </div>
      <div class="qr-card"><div class="qr">${qrSvg()}</div><div><div class="faint" style="font-size:12px">Einmal-Code</div><div class="code-otp">482 913</div></div><span class="faint num" style="font-size:12px">gültig noch 4:32 · danach neuer Code</span><button class="btn sm" data-act="toast" data-arg="Neuer Code erzeugt">${ic('redo', 'sm')}Neuer Code</button></div>
    </div></section>
    <section class="set-group"><h2>Gekoppelte Geräte <span class="faint num" style="font-weight:400;font-size:13px">${S.devices.length}</span></h2>
      ${S.devices.length ? S.devices.map((d, i) => `<div class="dev">${ic(d.icon, 'lg')}<div><b style="font-weight:500">${d.name}</b><span>${d.sub}</span></div><button class="btn sm danger" data-act="devdel" data-arg="${i}">Entfernen</button></div>`).join('') : '<p class="empty-line">Noch kein Gerät gekoppelt. Scanne den QR-Code oben.</p>'}
    </section>
    <section class="set-group"><h2>Unterwegs</h2>${srowHtml({ key: 'awake', lvl: 1, name: 'PC wach halten, solange ein Handy verbunden ist', ex: 'Verhindert, dass Standby die Verbindung trennt.', type: 'toggle', def: true }, false)}
      <div class="srow toggle-row"><div><div class="nm">Außerhalb des WLANs</div><div class="ex">Später optional über Tailscale. Der PC wird nie offen ins Internet gestellt.</div></div><div class="ctl"><span class="tag">Später</span></div><span></span></div></section>`;
}

/* ================= Ansicht: Erststart ================= */
const WIZ = ['Sprache & Ordner', 'Grafikkarte', 'Modell', 'Auto-Tune', 'Erster Chat'];
let engTimer = null;
function runEngineDownload() {
  clearInterval(engTimer);
  if (S.wiz.eng >= 100) return;
  engTimer = setInterval(() => {
    S.wiz.eng = Math.min(100, S.wiz.eng + 4);
    const bar = $('#engBar'), lbl = $('#engLbl'), nxt = $('#wizNext'), chk = $('#engChk');
    if (!bar) { clearInterval(engTimer); return; }
    bar.style.width = S.wiz.eng + '%';
    lbl.textContent = `${S.wiz.eng} % · ${Math.round(280 * S.wiz.eng / 100)} / 280 MB`;
    if (S.wiz.eng >= 100) { clearInterval(engTimer); chk.innerHTML = `${ic('check', 'sm')} Prüfsumme SHA-256 stimmt`; chk.style.color = 'var(--positive)'; nxt.disabled = false; }
  }, 110);
}
function vWizard(r) {
  const step = Math.min(5, Math.max(1, Number(r.a) || 1));
  const W = S.wiz;
  let title = '', lead = '', body = '', canNext = true, nextLbl = 'Weiter';
  if (step === 1) {
    title = 'Willkommen bei der RP-App'; lead = 'Wähle Sprache und Datenordner, dann suchen wir deine Grafikkarte und ein Modell. Bis zur ersten Antwort dauert es etwa fünf Minuten.';
    body = `<div class="wiz-card">
      <div class="field" style="margin:0"><label>Sprache</label><div class="seg" role="group"><button data-act="wlang" data-arg="Deutsch" aria-pressed="${W.lang === 'Deutsch'}">Deutsch</button><button data-act="wlang" data-arg="English" aria-pressed="${W.lang === 'English'}">English</button></div></div>
      <div><div style="font-weight:500;margin-bottom:6px">Datenordner</div><p class="muted" style="margin:0 0 8px;font-size:13px">Chats, Karten, Backups und der Kontext-Speicher. Vorschlag: das Laufwerk mit dem meisten Platz.</p>
        ${[['D', 'D:\\RP-App', 812, 1863, 'empfohlen'], ['C', 'C:\\Users\\…\\RP-App', 41, 476, 'wenig Platz']].map(([k, p, free, tot, note]) => `<label class="drive"><input type="radio" name="drive" data-act="wdrive" data-arg="${k}" ${W.drive === k ? 'checked' : ''}><div><b style="font-weight:500">${p}</b> <small class="num">· ${fmt(free)} GB frei · ${note}</small></div><div class="bar thin"><i style="width:${(1 - free / tot) * 100}%;background:var(--muted)"></i></div></label>`).join('')}
      </div></div>`;
  } else if (step === 2) {
    title = 'Grafikkarte gefunden'; lead = 'Wir laden die passende Rechen-Engine direkt von den offiziellen llama.cpp-Releases und prüfen die Prüfsumme.';
    canNext = W.eng >= 100;
    body = `<div class="wiz-card">
      <div class="gpu">${ic('chip')}<div><b>AMD Radeon RX 7900 XTX</b><span class="muted num">24 GB Grafikspeicher · Treiber 25.10.2</span></div><span class="tag badge-ok">ROCm wird unterstützt</span></div>
      <div class="dl"><div class="row"><span>llama.cpp · Windows-ROCm-Build</span><span class="num" id="engLbl">${W.eng} % · ${Math.round(280 * W.eng / 100)} / 280 MB</span></div>
        <div class="bar"><i id="engBar" style="width:${W.eng}%"></i></div>
        <div class="row"><span id="engChk" class="faint" ${W.eng >= 100 ? 'style="color:var(--positive)"' : ''}>${W.eng >= 100 ? `${ic('check', 'sm')} Prüfsumme SHA-256 stimmt` : 'Prüfsumme wird nach dem Download geprüft'}</span><span class="faint">Vulkan als Ersatz bereit</span></div></div>
    </div>`;
  } else if (step === 3) {
    title = 'Modell wählen'; lead = 'Wir haben auf deinem PC zwei Sprachmodelle gefunden. Du musst nichts herunterladen.';
    body = `<div class="opt-list">
      ${[['orion', 'Orion 26B-A4B · Q4_K_M · 15 GB', 'passt mit 128k Kontext · gefunden im LM-Studio-Ordner', 'empfohlen'], ['qwen', 'Qwen3.8 14B · IQ3_S · 6,9 GB', 'passt mit 128k · schneller, etwas schwächer im Rollenspiel', '']].map(([id, t, d, b]) => `<label class="opt"><input type="radio" name="model" data-act="wmodel" data-arg="${id}" ${W.model === id ? 'checked' : ''}><div><b>${t}</b><span>${d}</span></div>${b ? `<span class="tag badge-ok">${b}</span>` : '<span></span>'}</label>`).join('')}
      <label class="opt"><input type="radio" name="model" data-act="wmodel" data-arg="dl" ${W.model === 'dl' ? 'checked' : ''}><div><b>Empfohlenes Modell laden</b><span>Gemma 4 12B · Q8_0 · 12,6 GB · etwa 5 Minuten</span></div><span></span></label>
      <button class="btn sm ghost" style="justify-self:start" data-act="toast" data-arg="Ordner wählen …">${ic('folder', 'sm')}Weiteren Modellordner hinzufügen</button></div>`;
  } else if (step === 4) {
    title = 'Auto-Tune: schnellste Einstellung gemessen'; lead = 'Wir haben vier Varianten gemessen (1 Min 42 s). Das Ergebnis steht im Modellprofil und wird nach Engine-Updates neu geprüft.';
    body = `<div class="wiz-card"><div class="tune-wrap"><table class="data tune"><thead><tr><th>Rechenweg</th><th>KV-Cache</th><th class="num">tok/s leer</th><th class="num">tok/s 64k</th><th class="num">Kontext</th></tr></thead><tbody>
      ${TUNE.map(t => `<tr class="${t.best ? 'sel' : ''}"><td>${t.best ? '<span class="mark"></span>' : ''}${t.be}</td><td>${t.kv}</td><td class="num">${t.g0}</td><td class="num">${t.g64}</td><td class="num">${t.ctx}</td></tr>`).join('')}
      </tbody></table></div>
      <div class="well" style="font-size:14px"><b style="font-weight:600">Gewählt: ROCm mit KV q8_0 – 104 tok/s, 128k Kontext.</b><div class="muted" style="font-size:13px;margin-top:2px">f16 wäre 2 tok/s schneller, schafft aber nur 64k ohne Speicherüberlauf.</div></div></div>`;
  } else {
    title = 'Womit möchtest du anfangen?'; lead = 'Danach startet dein erster Chat. Alles Weitere findest du unter „Charaktere“.';
    nextLbl = 'Chat starten';
    body = `<div class="choice-grid">
      ${[['karte', 'upload', 'Karte importieren', 'PNG oder JSON – auch viele auf einmal.'], ['st', 'folder', 'SillyTavern übernehmen', 'Karten, Chats, Personas und Lorebooks mit einem Klick.'], ['beispiel', 'chars', 'Beispielkarte', 'Kapitänin Maren Holt – ein Abenteuer auf See.']].map(([id, i2, t, d]) => `<button class="choice" data-act="wstart" data-arg="${id}" aria-pressed="${W.start === id}">${ic(i2, 'lg')}<b>${t}</b><span>${d}</span></button>`).join('')}
    </div>`;
  }
  return `<div class="wiz"><div class="wiz-in">
    <div class="wiz-brand"><svg class="brand-mark" viewBox="0 0 26 26" aria-hidden="true"><rect width="26" height="26" rx="8" fill="var(--text)"/><path d="M8 18.5V7.5h5.2a3.4 3.4 0 0 1 0 6.8H8M13 14.3l4.8 4.2" fill="none" stroke="var(--ground)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg><b>RP-App</b><a href="#/chats">Erststart verlassen</a></div>
    <div class="stepper" aria-hidden="true">${WIZ.map((_, i) => `<i class="${i + 1 < step ? 'done' : i + 1 === step ? 'cur' : ''}"></i>`).join('')}</div>
    <div class="step-lbls">${WIZ.map((l, i) => `<span class="${i + 1 === step ? 'cur' : ''}">${l}</span>`).join('')}</div>
    <p class="faint num" style="margin:0 0 6px;font-size:13px">Schritt ${step} von 5</p>
    <h1>${title}</h1><p class="lead">${lead}</p>
    ${body}
    <div class="wiz-foot">
      ${step > 1 ? `<a class="btn ghost" href="#/erststart/${step - 1}">${ic('left', 'sm')}Zurück</a>` : ''}
      ${step === 4 ? `<a class="btn ghost" href="#/erststart/5">Überspringen</a>` : ''}
      <button class="btn primary sp" id="wizNext" data-act="wnext" data-arg="${step}" ${canNext ? '' : 'disabled'}>${nextLbl}${ic('right', 'sm')}</button>
    </div>
  </div></div>`;
}

/* ================= Befehlsleiste ================= */
const CMDS = [
  ['Weiter, wo ich war: Sturm vor Kap Morn', 'Chats', 'chats', () => go('#/chat/c1')],
  ['Alle Chats', 'Gehe zu', 'chats', () => go('#/chats')],
  ['Charaktere', 'Gehe zu', 'chars', () => go('#/charaktere')],
  ['Neuer Chat mit Ilvaris Telmeren', 'Chats', 'plus', () => go('#/chat/c2')],
  ['Modelle', 'Gehe zu', 'discover', () => go('#/entdecken/modelle')],
  ['Einstellungen', 'Gehe zu', 'settings', () => go('#/einstellungen/allgemein')],
  ['Prompt-Inspektor öffnen', 'Chat', 'layers', () => openInspector()],
  ['Szene-Leiste ein/aus', 'Chat', 'scene', () => { S.scene = !S.scene; S.sceneOpen = !S.sceneOpen; go('#/chat/c1', true); }],
  ['Story-Modus umschalten', 'Chat', 'book', () => { S.story = !S.story; go('#/chat/c1', true); }],
  ['Hell/Dunkel wechseln', 'Ansicht', 'sun', () => toggleTheme()],
  ['Handy koppeln', 'Einstellungen', 'phone', () => go('#/einstellungen/handy')],
  ['Kreativität (Temperature)', 'Einstellungen', 'settings', () => { S.level = Math.max(S.level, 2); go('#/einstellungen/chat'); }],
  ['Karte importieren', 'Charaktere', 'upload', () => go('#/charaktere')],
  ['Karten-Check: Maren Holt', 'Charaktere', 'check', () => go('#/charakter/maren/check')],
  ['Modell wechseln: Qwen3.8 14B', 'Modell', 'chip', () => toast('Qwen3.8 14B wird geladen …')],
  ['Erststart anzeigen', 'Hilfe', 'info', () => go('#/erststart/1')],
];
let cmdSel = 0, cmdList = CMDS;
function go(h, force) { if (location.hash === h && force) render(); else location.hash = h; }
function openCmdk() {
  closeAll(); $('#cmdk').classList.add('on'); $('#backdrop').classList.add('on');
  const i = $('#cmdkInput'); i.value = ''; cmdFilter(''); setTimeout(() => i.focus(), 30);
}
function cmdFilter(q) {
  q = q.toLowerCase(); cmdList = CMDS.filter(c => (c[0] + ' ' + c[1]).toLowerCase().includes(q)); cmdSel = 0;
  $('#cmdkList').innerHTML = cmdList.length ? cmdList.map((c, i) => `<li role="option" data-ci="${i}" class="${i === cmdSel ? 'sel' : ''}" aria-selected="${i === cmdSel}">${ic(c[2], 'sm')}<span>${esc(c[0])}</span><span class="grp">${c[1]}</span></li>`).join('') : `<li class="none">Kein Befehl passt. Versuch „Chat“, „Modell“ oder „Handy“.</li>`;
}
function cmdMove(d) { if (!cmdList.length) return; cmdSel = (cmdSel + d + cmdList.length) % cmdList.length; $$('#cmdkList li').forEach((li, i) => { li.classList.toggle('sel', i === cmdSel); li.setAttribute('aria-selected', i === cmdSel); }); const s = $('#cmdkList li.sel'); s && s.scrollIntoView({ block: 'nearest' }); }
function cmdRun(i) { const c = cmdList[i]; if (!c) return; closeAll(); c[3](); }

function openInspector() { closeAll(); const d = $('#inspector'); d.innerHTML = inspectorHtml(); d.classList.add('on'); d.setAttribute('aria-hidden', 'false'); $('#backdrop').classList.add('on'); }
function closeAll() {
  $('#cmdk').classList.remove('on'); const d = $('#inspector'); d.classList.remove('on'); d.setAttribute('aria-hidden', 'true');
  $('#backdrop').classList.remove('on');
  if (S.sceneOpen) { S.sceneOpen = false; const cr = $('#chatRoot'); cr && cr.classList.remove('scene-open'); }
}
function toggleTheme() {
  S.theme = S.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = S.theme; store.set('rp-theme', S.theme);
  S.set.theme = S.theme === 'dark' ? 'Dunkel' : 'Hell';
  renderChrome();
  if (parse().name === 'einstellungen') render();
}
const isNarrow = () => window.matchMedia('(max-width: 1023px)').matches;

/* ================= Events ================= */
document.addEventListener('click', e => {
  const li = e.target.closest('#cmdkList li[data-ci]'); if (li) { cmdRun(Number(li.dataset.ci)); return; }
  const el = e.target.closest('[data-act]'); if (!el) return;
  const act = el.dataset.act, arg = el.dataset.arg;
  if (el.tagName === 'BUTTON' && el.closest('a')) e.preventDefault();
  const msg = id => S.msgs.find(m => m.id === id);
  switch (act) {
    case 'cmdk': openCmdk(); break;
    case 'inspector': openInspector(); break;
    case 'closeAll': closeAll(); break;
    case 'theme': toggleTheme(); break;
    case 'toast': toast(esc(arg)); break;
    case 'undo': if (toast.undo) { toast.undo(); toast.undo = null; $('#toast').classList.remove('on'); } break;
    // Chats / Charaktere
    case 'chattag': S.chatTag = arg; render(); break;
    case 'charf': S.charFilter = arg; $$('[data-act="charf"]').forEach(b => b.setAttribute('aria-pressed', b.dataset.arg === arg)); $('#charGrid').innerHTML = charGridHtml(); break;
    case 'fav': e.preventDefault(); S.favs[arg] = !S.favs[arg]; el.setAttribute('aria-pressed', S.favs[arg]); toast(S.favs[arg] ? 'Zu Favoriten hinzugefügt' : 'Aus Favoriten entfernt'); break;
    case 'greet': { const f = fieldsFor(charById(parse().a)); S.greet = (Math.min(S.greet, f.greets.length - 1) + Number(arg) + f.greets.length) % f.greets.length; render(); break; }
    // Chat
    case 'mode': S.story = arg === 'story'; rerenderChat(); break;
    case 'scene': if (isNarrow()) { S.sceneOpen = true; $('#chatRoot').classList.add('scene-open'); $('#backdrop').classList.add('on'); } else { S.scene = !S.scene; rerenderChat(); } break;
    case 'sceneClose': closeAll(); break;
    case 'intmode': S.intMode = arg; rerenderScene(); break;
    case 'int': S.intensity = Number(arg); if (S.intMode === 'Auto') S.intMode = 'Manuell'; rerenderScene(); break;
    case 'len': S.length = arg; rerenderScene(); break;
    case 'lb': S.lb[arg] = !S.lb[arg]; rerenderScene(); break;
    case 'swipe': { const [id, d] = arg.split('|'); doSwipe(id, Number(d)); break; }
    case 'm-edit': S.editing = arg; S.info = null; rerenderChat(); const ta = $('.edit-box textarea'); ta && ta.focus(); break;
    case 'edit-cancel': S.editing = null; rerenderChat(); break;
    case 'edit-save': { const m = msg(arg); const v = $('.edit-box textarea').value; if (m.variants) m.variants[m.sw] = v; else m.text = v; S.editing = null; rerenderChat(); toast('Gespeichert'); break; }
    case 'm-copy': try { navigator.clipboard && navigator.clipboard.writeText(msgText(msg(arg)).replace(/\*/g, '')); } catch (x) { /* Datei-Modus */ } toast('Kopiert'); break;
    case 'm-regen': { const m = msg(arg); if (!m.variants) { toast('„Neu“ ist bei Begrüßungen das Wischen'); break; } m.variants.push(''); m.swN = m.variants.length; m.sw = m.variants.length - 1; stream(m, LAST_VARIANTS[m.sw % LAST_VARIANTS.length]); break; }
    case 'm-cont': { const m = msg(arg); const base = msgText(m); stream(m, base + '\n\n*Hinter ihr schlägt die Tür gegen den Rahmen, und für einen Moment hörst du nur noch den Sturm – und ihre Schritte auf dem Niedergang.*'); break; }
    case 'm-branch': toast('Branch „Sturm vor Kap Morn ⑂ 2“ angelegt – ab dieser Nachricht'); break;
    case 'm-hide': { const m = msg(arg); m.hidden = !m.hidden; rerenderChat(); toast(m.hidden ? 'Aus dem Kontext ausgeblendet' : 'Wieder im Kontext'); break; }
    case 'm-del': {
      const i = S.msgs.findIndex(m => m.id === arg); const [gone] = S.msgs.splice(i, 1); rerenderChat();
      toast('Nachricht gelöscht', { undo: () => { S.msgs.splice(i, 0, gone); rerenderChat(); } }); break;
    }
    case 'm-info': S.info = S.info === arg ? null : arg; rerenderChat(); break;
    case 'fix': { const m = S.msgs[S.msgs.length - 1]; if (!m.variants) break; m.variants.push(''); m.swN = m.variants.length; m.sw = m.variants.length - 1; toast(`Neu geschrieben: ${arg}`); stream(m, FIX_TEXT[arg]); break; }
    case 'sugg': { const t = $('#composer'); t.value = SUGGESTIONS[Number(arg)]; autosize(t); t.focus(); break; }
    case 'imp': typeInto($('#composer'), IMPERSONATE); break;
    case 'regie': S.regie = !S.regie; rerenderChat(); if (S.regie) { const i = $('.regie input'); i && i.focus(); } break;
    case 'send': sendMsg(); break;
    case 'stop': stopStream(false); break;
    // Entdecken
    case 'msel': S.modelSel = arg; render(); break;
    case 'dlpause': S.loadPaused = !S.loadPaused; render(); break;
    // Einstellungen
    case 'lvl': S.level = Number(arg); store.set('rp-level', S.level); render(); break;
    case 'set': S.set[el.dataset.key] = el.dataset.val; afterSet(el.dataset.key); break;
    case 'settoggle': S.set[el.dataset.key] = !S.set[el.dataset.key]; afterSet(el.dataset.key); break;
    case 'reset': { const r = ALL_ROWS.find(x => x.key === el.dataset.key) || { def: true }; S.set[el.dataset.key] = r.def; afterSet(el.dataset.key); toast('Auf Standard zurückgesetzt'); break; }
    case 'devdel': { const i = Number(arg); const [d] = S.devices.splice(i, 1); render(); toast(`${d.name} entfernt`, { undo: () => { S.devices.splice(i, 0, d); render(); } }); break; }
    // Erststart
    case 'wlang': S.wiz.lang = arg; render(); break;
    case 'wdrive': S.wiz.drive = arg; break;
    case 'wmodel': S.wiz.model = arg; break;
    case 'wstart': S.wiz.start = arg; $$('[data-act="wstart"]').forEach(b => b.setAttribute('aria-pressed', b.dataset.arg === arg)); break;
    case 'wnext': { const s = Number(arg); if (s < 5) go(`#/erststart/${s + 1}`); else { go('#/chat/c1'); toast('Erster Chat mit Maren Holt gestartet'); } break; }
  }
});
function afterSet(key) {
  if (key === 'theme') { const want = S.set.theme === 'Hell' ? 'light' : 'dark'; if (want !== S.theme) { toggleTheme(); return; } }
  if (key === 'fontsize') renderChrome();
  const page = SETTINGS.find(p => p.id === parse().a) || SETTINGS[0];
  $('#setBody').innerHTML = setBodyHtml(page);
}
function rerenderScene() { const sc = $('.scene'); if (!sc) return; const open = S.sceneOpen; sc.outerHTML = sceneHtml(); const h = $('.hint .num'); if (h) h.textContent = `Antwort ≈ ${{ Kurz: 180, Mittel: 350, Lang: 700 }[S.length]} Tokens · Intensität ${S.intMode === 'Aus' ? 'aus' : S.intensity}`; if (open) $('#chatRoot').classList.add('scene-open'); }
function sendMsg() {
  const t = $('#composer'); const v = t.value.trim();
  if (!v) { toast('Erst etwas schreiben – oder einen Vorschlag antippen'); return; }
  const now = new Date(); const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
  S.msgs.forEach(m => { if (m.variants) m.think = null; });
  S.info = null;
  S.msgs.push({ id: 'u' + Date.now(), who: 'user', time, text: v, tokens: tok(v) });
  t.value = '';
  if (S.regie && S.regieText) toast('Regie-Anweisung gilt für diese Antwort');
  S.regie = false; S.regieText = '';
  const m = { id: 'a' + Date.now(), who: 'char', time, variants: [''], sw: 0, swN: 1, tokens: 168, tps: 104.3 };
  S.msgs.push(m); stream(m, REPLY_TEXT);
}
document.addEventListener('input', e => {
  const t = e.target;
  if (t.id === 'cmdkInput') { cmdFilter(t.value); return; }
  if (t.id === 'composer') { autosize(t); return; }
  const k = t.dataset.in;
  if (k === 'chatq') { S.chatQuery = t.value; const list = filteredChats(); $('#chatListWrap').innerHTML = chatListHtml(list); const right = $('.toolbar .right'); if (right) right.textContent = `${list.length} von ${CHATS.length}`; }
  if (k === 'charq') { S.charQuery = t.value; $('#charGrid').innerHTML = charGridHtml(); }
  if (k === 'setq') { S.setQuery = t.value; const page = SETTINGS.find(p => p.id === parse().a) || SETTINGS[0]; $('#setBody').innerHTML = setBodyHtml(page); $$('.set-nav a').forEach(a => a.toggleAttribute('aria-current', false)); if (!S.setQuery) render(); }
  if (k === 'count') { const s = t.closest('.field').querySelector('[data-count]'); if (s) s.textContent = `${fmt(tok(t.value))} Tokens`; }
  if (k === 'loretest') { S.loreTest = t.value; const tmp = document.createElement('div'); tmp.innerHTML = loreHtml(); $('#loreTable').innerHTML = $('#loreTable', tmp).innerHTML; $('#loreSum').textContent = $('#loreSum', tmp).textContent; }
  if (k === 'regie') S.regieText = t.value;
  if (t.type === 'range' && t.dataset.key) { S.set[t.dataset.key] = Number(t.value); t.nextElementSibling.textContent = String(t.value).replace('.', ','); t.closest('.srow').classList.toggle('changed', S.set[t.dataset.key] !== (ALL_ROWS.find(r => r.key === t.dataset.key) || {}).def); if (t.dataset.key === 'fontsize') renderChrome(); }
});
document.addEventListener('change', e => {
  const t = e.target;
  if (t.matches('select[data-key]')) { S.set[t.dataset.key] = t.value; afterSet(t.dataset.key); }
  if (t.matches('input.input[data-key], textarea[data-key]')) { S.set[t.dataset.key] = t.value; }
});
document.addEventListener('keydown', e => {
  const cmdOpen = $('#cmdk').classList.contains('on');
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); cmdOpen ? closeAll() : openCmdk(); return; }
  if (e.key === 'Escape') { closeAll(); return; }
  if (cmdOpen) {
    if (e.key === 'ArrowDown') { e.preventDefault(); cmdMove(1); }
    if (e.key === 'ArrowUp') { e.preventDefault(); cmdMove(-1); }
    if (e.key === 'Enter') { e.preventDefault(); cmdRun(cmdSel); }
    return;
  }
  if (e.target.id === 'composer' && e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); S.streaming ? null : sendMsg(); }
  if (e.target.matches('.m-row[data-act]') && e.key === 'Enter') e.target.click();
});

/* Drag & Drop auf die Import-Zone */
document.addEventListener('dragover', e => { const z = e.target.closest && e.target.closest('.dropzone'); if (z) { e.preventDefault(); z.classList.add('over'); } });
document.addEventListener('dragleave', e => { const z = e.target.closest && e.target.closest('.dropzone'); if (z) z.classList.remove('over'); });
document.addEventListener('drop', e => {
  const z = e.target.closest && e.target.closest('.dropzone'); if (!z) return; e.preventDefault(); z.classList.remove('over');
  const n = (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length) || 0;
  toast(n ? `${n} Datei${n > 1 ? 'en' : ''} erkannt – Prototyp: es wird nichts importiert` : 'Nichts erkannt');
});

window.addEventListener('hashchange', () => { S.editing = null; S.info = null; S.sceneOpen = false; closeAll(); render(); });
renderChrome();
render();
