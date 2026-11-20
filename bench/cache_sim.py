"""Phase 0 measurement tool for Lorelamp.

Simulates a multi-turn roleplay chat against a running llama-server and records, per turn,
how many prompt tokens had to be (re)processed vs. reused from the KV cache.

Layouts (how the prompt is assembled each turn):
  A  SillyTavern-like: triggered lorebook entries in the HEAD (before the card),
     author's note injected 4 messages deep, history trimmed one message at a time.
  B  Head / history / tail: static head, triggered lore + author's note only in the
     final user message (tail), history trimmed in 25 % blocks (first message pinned).
  C  Append-only: like B, but the tail notes stay in the history permanently.

Also: `--depth` mode measures prefill and generation speed at several context sizes.

Only the Python standard library is used. All sample content is SFW.
"""
import argparse
import csv
import json
import statistics
import time
import urllib.request
from pathlib import Path

RAW = Path(__file__).parent / "raw"

SYSTEM = (
    "Du bist ein erfahrener Erzähler für interaktive Geschichten. Schreibe auf Deutsch, "
    "in der dritten Person, lebendig und bildhaft. Handle und sprich niemals für {{user}}. "
    "Halte Antworten bei 2–4 Absätzen. Zeige Gefühle durch Handlungen statt sie zu benennen."
)
CARD = (
    "Name: Kapitänin Mara Voss\nAlter: 34\n"
    "Beschreibung: Mara führt den Frachtsegler 'Silbermöwe' durch die Inselketten des Nordmeers. "
    "Sie ist groß, wettergegerbt, trägt einen abgewetzten Ledermantel und eine Messingtaschenuhr ihres Vaters. "
    "Ihr Humor ist trocken, ihr Blick wachsam. Sie hasst Schmuggler, seit einer ihren Bruder verraten hat.\n"
    "Persönlichkeit: pragmatisch, loyal, stur, heimlich romantisch, spricht knapp, lacht selten aber laut.\n"
    "Szenario: {{user}} hat als Kartograph auf der Silbermöwe angeheuert. Ein Sturm zieht auf, "
    "und im Laderaum liegt eine versiegelte Kiste, die niemand angemeldet hat."
)
EXAMPLES = (
    "Beispiel:\n{{user}}: Wie lange noch bis zum Hafen?\n"
    "Mara: *Sie klappt die Taschenuhr auf, ohne hinzusehen.* \"Zwei Glasen, wenn der Wind hält. "
    "Drei, wenn du weiter fragst.\" *Ein Mundwinkel zuckt.*"
)
LORE = {
    "hafen": "Lorebook – Hafen Kalmbrück: Nebliger Handelshafen, Zöllner sind bestechlich, das Wirtshaus 'Zum Anker' gehört Maras alter Freundin Ilsa.",
    "sturm": "Lorebook – Nordmeerstürme: Kommen schnell aus Nordwest, Wellen bis zehn Meter, die Silbermöwe hält sie aus, wenn die Ladung festgezurrt ist.",
    "karte": "Lorebook – Seekarten: Die alten Karten der Inselkette sind ungenau; zwischen Grauklippe und Ostfeuer liegt ein unkartiertes Riff.",
    "kristall": "Lorebook – Leuchtkristalle: Seltene Kristalle, die im Dunkeln blau glühen; ihr Handel ist verboten, Schmuggler zahlen ein Vermögen.",
    "schmuggler": "Lorebook – Die Graue Hand: Schmugglerbande im Nordmeer, erkennbar an einer grauen Tätowierung am Handgelenk.",
}
AUTHORS_NOTE = "[Hinweis des Autors: Die Stimmung bleibt angespannt, der Sturm wird stärker.]"
USER_LINES = [
    "Ich trete neben Mara an die Reling und schaue auf die dunklen Wolken.",
    "Wie schlimm wird der Sturm diesmal?",
    "Ich zeige ihr die alte Karte und deute auf die Lücke zwischen den Inseln.",
    "Sollten wir nicht lieber den nächsten Hafen anlaufen?",
    "Ich frage leise, ob sie weiß, was in der versiegelten Kiste ist.",
    "Ich helfe der Mannschaft, die Ladung festzuzurren.",
    "Ich bemerke ein blaues Glimmen durch einen Spalt der Kiste.",
    "Ist das etwa ein Kristall?",
    "Ich sage ihr, dass ich gestern einen Mann mit grauer Tätowierung gesehen habe.",
    "Ich halte mich am Mast fest, als die erste große Welle kommt.",
    "Ich rufe ihr zu, ob sie Hilfe am Steuer braucht.",
    "Ich lache nervös und sage, dass ich Kartograph bin, kein Seemann.",
    "Ich frage nach ihrem Bruder.",
    "Ich reiche ihr einen Becher heißen Tee aus der Kombüse.",
    "Ich schlage vor, die Kiste über Bord zu werfen.",
    "Ich frage, ob die Schmuggler uns verfolgen könnten.",
    "Ich zeichne im Licht der Laterne das Riff auf die Karte ein.",
    "Ich frage, wer Ilsa ist.",
    "Ich schaue ihr in die Augen und sage, dass ich ihr vertraue.",
    "Ich höre ein fremdes Schiffshorn im Sturm.",
    "Ich frage, ob wir den Hafen vor Mitternacht erreichen.",
    "Ich öffne vorsichtig die Kiste.",
    "Ich halte den Kristall gegen das Licht.",
    "Ich frage, was ein Zöllner in Kalmbrück für Schweigen verlangt.",
    "Ich lehne mich erschöpft gegen die Kajütenwand.",
    "Ich frage, ob sie jemals ans Aufhören gedacht hat.",
    "Ich zeige auf das zweite Schiff am Horizont.",
    "Ich frage, ob die Graue Hand an Bord einen Spitzel hat.",
    "Ich sage, dass der Sturm endlich nachlässt.",
    "Ich frage, was sie tun wird, wenn wir angelegt haben.",
]
FILL = [
    "Die Silbermöwe ächzt unter dem Druck der Wellen, Salzwasser spritzt über das Deck.",
    "Mara prüft die Taue mit geübten Griffen und ruft der Mannschaft knappe Befehle zu.",
    "Im Laderaum riecht es nach Teer, nassem Holz und altem Tauwerk.",
    "Die Laterne schwingt am Haken und wirft unruhige Schatten über die Karten.",
    "Ein Möwenschwarm kreist über dem Mast, als suche er Schutz vor dem Wetter.",
    "Der Steuermann murmelt ein altes Seemannsgebet und hält den Kurs.",
    "Irgendwo unter Deck klappert eine lose Pfanne im Takt der Dünung.",
    "Mara wischt sich das Wasser aus dem Gesicht und lacht kurz, rau und ehrlich.",
    "Die Sterne verschwinden hinter einer Wand aus grauem Nebel.",
    "Der Wind dreht, und die Segel schlagen laut gegen die Rahen.",
]


def post(base, path, body, timeout=900):
    req = urllib.request.Request(base + path, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def ntok(base, text, cache):
    if text not in cache:
        cache[text] = len(post(base, "/tokenize", {"content": text})["tokens"])
    return cache[text]


def synthetic_history(n_msgs):
    msgs = []
    for i in range(n_msgs):
        sents = [FILL[(i * 3 + k) % len(FILL)] for k in range(6)]
        role = "user" if i % 2 == 0 else "assistant"
        prefix = f"(Rückblick {i + 1}) "
        msgs.append({"role": role, "content": prefix + " ".join(sents)})
    if msgs[0]["role"] != "user":
        msgs.insert(0, {"role": "user", "content": "(Beginn der Reise)"})
    return msgs


def triggered(text_window):
    low = text_window.lower()
    return [v for k, v in LORE.items() if k in low]


def fit_history(base, hist, budget, cache, mode, state=None):
    """Return the slice of history that fits the token budget.
    mode 'single': drop oldest messages one by one (SillyTavern style).
    mode 'block':  keep first message, drop oldest ~25 % of the budget at once; the cut
                   position is sticky (state['start']) so the prefix stays stable until the
                   budget is exceeded again."""
    total = sum(ntok(base, m["content"], cache) for m in hist)
    if total <= budget:
        return hist
    if mode == "single":
        out = list(hist)
        while sum(ntok(base, m["content"], cache) for m in out) > budget and len(out) > 2:
            out.pop(0)
            if out[0]["role"] != "user":
                out.pop(0)
        return out
    first = hist[0]
    start = state.get("start", 1)
    rest = hist[start:]
    if sum(ntok(base, m["content"], cache) for m in [first] + rest) > budget:
        target = budget * 0.75
        while sum(ntok(base, m["content"], cache) for m in [first] + rest) > target and len(rest) > 2:
            start += 1
            rest = hist[start:]
        if rest and rest[0]["role"] != "assistant":
            start += 1
            rest = hist[start:]
        state["start"] = start
    return [first] + rest


def build(layout, base, hist, user_msg, budget, cache, state):
    window = " ".join(m["content"] for m in hist[-4:]) + " " + user_msg
    lore_now = triggered(window)
    head = SYSTEM + "\n\n" + CARD + "\n\n" + EXAMPLES
    if layout == "A":
        head = SYSTEM + "\n\n" + ("\n".join(lore_now) + "\n\n" if lore_now else "") + CARD + "\n\n" + EXAMPLES
        h = fit_history(base, hist, budget, cache, "single")
        h = [dict(m) for m in h]
        users = [i for i, m in enumerate(h) if m["role"] == "user"]
        if len(users) >= 2:
            i = users[-2]
            h[i]["content"] = h[i]["content"] + "\n" + AUTHORS_NOTE
        msgs = [{"role": "system", "content": head}] + h + [{"role": "user", "content": user_msg}]
        return msgs, user_msg
    h = fit_history(base, hist, budget, cache, "block", state)
    tail = ("\n".join(lore_now) + "\n" if lore_now else "") + AUTHORS_NOTE
    last = user_msg + "\n\n" + tail
    msgs = [{"role": "system", "content": head}] + h + [{"role": "user", "content": last}]
    stored_user = last if layout == "C" else user_msg
    return msgs, stored_user


def run_layout(args, layout):
    base, cache, state = args.url, {}, {}
    hist = synthetic_history(args.history_msgs)
    rows = []
    t_all = time.time()
    for turn, line in enumerate(USER_LINES[: args.turns], 1):
        msgs, stored_user = build(layout, base, hist, line, args.budget, cache, state)
        prompt = post(base, "/apply-template", {"messages": msgs, "chat_template_kwargs": {"enable_thinking": False}})["prompt"]
        t0 = time.time()
        res = post(base, "/completion", {"prompt": prompt, "n_predict": args.n_predict, "temperature": 0.8,
                                         "seed": 1000 + turn, "cache_prompt": True})
        wall = time.time() - t0
        tm = res.get("timings", {})
        reply = res.get("content", "").strip() or "(leer)"
        hist += [{"role": "user", "content": stored_user}, {"role": "assistant", "content": reply}]
        p_n, c_n = tm.get("prompt_n", 0), tm.get("cache_n", res.get("tokens_cached", 0))
        rows.append({"layout": layout, "turn": turn, "prompt_tokens_total": p_n + c_n, "processed": p_n, "cached": c_n,
                     "prompt_ms": round(tm.get("prompt_ms", 0)), "gen_tps": round(tm.get("predicted_per_second", 0), 1),
                     "wall_s": round(wall, 2)})
        r = rows[-1]
        print(f"  {layout} Zug {turn:2d}: Prompt {r['prompt_tokens_total']:6d} Tok · neu berechnet {p_n:6d} · "
              f"aus Cache {c_n:6d} · Einlesen {r['prompt_ms'] / 1000:5.1f}s · {r['gen_tps']:5.1f} tok/s", flush=True)
    RAW.mkdir(exist_ok=True)
    out = RAW / f"cache_{layout}.csv"
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    steady = rows[1:]
    proc = [r["processed"] for r in steady]
    tot = [r["prompt_tokens_total"] for r in steady]
    hit = 100 * (1 - sum(proc) / max(1, sum(tot)))
    print(f"== Aufbau {layout}: Cache-Quote {hit:.1f} % · Median neu berechnet {statistics.median(proc):.0f} Tok/Zug · "
          f"Einlesezeit gesamt {sum(r['prompt_ms'] for r in steady) / 1000:.1f}s · "
          f"Gen Median {statistics.median(r['gen_tps'] for r in steady):.1f} tok/s · Laufzeit {time.time() - t_all:.0f}s")


def run_depth(args):
    base = args.url
    rows = []
    for target in args.depths:
        words = []
        i = 0
        while len(words) < target * 0.75:
            words += FILL[i % len(FILL)].split() + [f"[{target}-{i}]"]
            i += 1
        text = " ".join(words)
        msgs = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": text + "\n\nFasse die Szene in drei Sätzen zusammen."}]
        prompt = post(base, "/apply-template", {"messages": msgs, "chat_template_kwargs": {"enable_thinking": False}})["prompt"]
        res = post(base, "/completion", {"prompt": prompt, "n_predict": 128, "temperature": 0.7, "seed": 7,
                                         "cache_prompt": False, "ignore_eos": True})
        tm = res["timings"]
        rows.append({"ziel": target, "prompt_n": tm["prompt_n"], "einlesen_tps": round(tm["prompt_per_second"]),
                     "einlesen_s": round(tm["prompt_ms"] / 1000, 1), "gen_tps": round(tm["predicted_per_second"], 1)})
        r = rows[-1]
        print(f"  Kontext {r['prompt_n']:6d} Tok: Einlesen {r['einlesen_tps']:5d} tok/s ({r['einlesen_s']:5.1f}s) · "
              f"Generierung {r['gen_tps']:5.1f} tok/s", flush=True)
    RAW.mkdir(exist_ok=True)
    (RAW / "depth.json").write_text(json.dumps(rows, indent=1), encoding="utf-8")


def run_slots(args):
    """Fill the slot with a long prompt, save it to disk, erase it, restore it, then ask again
    with a small addition and check how many tokens had to be recomputed."""
    base = args.url
    words, i = [], 0
    while len(words) < args.slot_words:
        words += FILL[i % len(FILL)].split() + [f"[s-{i}]"]
        i += 1
    msgs = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": " ".join(words)}]
    prompt = post(base, "/apply-template", {"messages": msgs, "chat_template_kwargs": {"enable_thinking": False}})["prompt"]
    body = {"prompt": prompt, "n_predict": 16, "temperature": 0.7, "seed": 3, "cache_prompt": True}
    t = post(base, "/completion", body)["timings"]
    print(f"  Füllen: {t['prompt_n']} Tok in {t['prompt_ms'] / 1000:.1f}s")
    t0 = time.time()
    r = post(base, "/slots/0?action=save", {"filename": "rp-app-test.bin"})
    print(f"  Speichern: {time.time() - t0:.1f}s · {r.get('n_saved')} Tok · Antwort {json.dumps(r)[:160]}")
    post(base, "/slots/0?action=erase", {})
    t0 = time.time()
    r = post(base, "/slots/0?action=restore", {"filename": "rp-app-test.bin"})
    print(f"  Laden: {time.time() - t0:.1f}s · {r.get('n_restored')} Tok")
    body["prompt"] = prompt + " Und weiter."
    t = post(base, "/completion", body)["timings"]
    print(f"  Danach neu berechnet: {t['prompt_n']} Tok · aus Cache {t.get('cache_n')} · Einlesen {t['prompt_ms'] / 1000:.1f}s")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:8080")
    ap.add_argument("--layout", choices=["A", "B", "C"])
    ap.add_argument("--depth", action="store_true")
    ap.add_argument("--slots", action="store_true")
    ap.add_argument("--slot-words", type=int, default=30000)
    ap.add_argument("--depths", type=int, nargs="+", default=[1000, 8000, 32000, 64000])
    ap.add_argument("--turns", type=int, default=30)
    ap.add_argument("--n-predict", type=int, default=120)
    ap.add_argument("--budget", type=int, default=12000, help="token budget for chat history")
    ap.add_argument("--history-msgs", type=int, default=90)
    args = ap.parse_args()
    if args.depth:
        run_depth(args)
    elif args.slots:
        run_slots(args)
    elif args.layout:
        run_layout(args, args.layout)
    else:
        ap.error("--layout A|B|C oder --depth")


if __name__ == "__main__":
    main()
