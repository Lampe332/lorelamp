//! "Was willst du mit dem Modell machen?": the catalog ordered for a purpose (the needs read from
//! the user's sentence, `rp_core::purpose`) on a PC, each model with the reasons that put it
//! there. Pure, like `pick.rs`: the same PC, catalog and needs always give the same order.
//!
//! The score starts from the app's quality (0.6 × roleplay + 0.4 × German, the owner's rule) and
//! adds one term per need; an installed file counts [`pick::INSTALLED_BONUS`] more and a measured
//! speed or context replaces the estimate. Every term that moved a model up is a reason the UI
//! shows; a term that pulled it down is a caveat.

use std::collections::HashSet;

use rp_core::purpose::Need;
use serde::Serialize;

use super::pick::{self, candidate, gb1, Cand, Fit, Measured, Pc, Sized, NAMES};

/// Points per German rating step above or below 3 when German is asked for.
pub const GERMAN_STEP: f64 = 0.6;
/// Points per roleplay rating step around 3 for "quality" (groups and long replies: a part of it).
pub const QUALITY_STEP: f64 = 0.5;
pub const GROUP_RP_STEP: f64 = 0.3;
pub const REPLY_RP_STEP: f64 = 0.2;
/// A model made for stories or adventures (its card says so) when narration is asked for.
pub const STORY_BONUS: f64 = 1.3;
/// Points per doubling of the context above 8k: "much context", long chats (up to 64k), groups (up to 32k).
pub const CONTEXT_STEP: f64 = 0.4;
pub const LONG_CHAT_STEP: f64 = 0.3;
pub const GROUP_CTX_STEP: f64 = 0.3;
/// A model that never refuses, when "uncensored" is asked for (those that refuse are left out).
pub const FREE_BONUS: f64 = 0.3;
/// "Fast" counts the speed term this many times: whoever asks for speed wants the fastest model
/// that is good enough, not the best one that is fast enough.
pub const FAST_WEIGHT: f64 = 2.0;
/// A weak PC counts the speed this much, and wants a light model: all on the graphics card, or
/// without one at most [`LIGHT_RAM_SHARE`] of what the RAM may hold.
pub const WEAK_SPEED_SHARE: f64 = 0.6;
pub const LIGHT_BONUS: f64 = 0.3;
pub const LIGHT_RAM_SHARE: f64 = 0.5;
/// A model that is not light, on a weak PC (as long as a light one fits).
pub const HEAVY_COST: f64 = 3.0;
/// A part in RAM next to a graphics card costs speed: this much less, always; and this much when
/// speed is asked for.
pub const RAM_HELP_COST: f64 = 0.4;
pub const RAM_HELP_COST_FAST: f64 = 1.5;
/// The speed term reaches from −[`SPEED_SPAN`] to +[`SPEED_SPAN`].
pub const SPEED_SPAN: f64 = 1.5;
/// Suggestions shown as the answer; base lines are not repeated among them while others are near.
pub const TOP: usize = 5;
/// A model of another base line may stand this far below and still be shown before a second one
/// of a line that is there already.
pub const TOP_SLACK: f64 = 0.8;

const GB: f64 = 1_073_741_824.0;

/// One reason (or caveat) behind a suggestion; `value` is the number the sentence needs (a rating,
/// tokens of context, tokens/s, billions of parameters).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Why {
    pub code: &'static str,
    pub value: Option<f64>,
    /// How many points it moved the model (negative: a caveat).
    pub points: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub tier: &'static str,
    /// Quality plus the terms of the needs, two decimals.
    pub score: f64,
    pub q: f64,
    pub installed: bool,
    pub measured: Option<Measured>,
    pub fit: Fit,
    pub n_cpu_moe: u64,
    pub gpu_gb: f64,
    pub ram_gb: f64,
    pub est_gb: f64,
    pub ctx_max: u64,
    /// What speaks for it, the strongest first (at most three).
    pub reasons: Vec<Why>,
    /// What speaks against it for this purpose (at most two).
    pub caveats: Vec<Why>,
}

/// From −[`SPEED_SPAN`] (very slow) to +[`SPEED_SPAN`]: a measurement when there is one (10 tokens/s
/// is the middle, each doubling half a point), else the bytes read per token (2 GB on the
/// graphics card is the top, each doubling half a point less; RAM bytes count ten times).
fn speed_term(c: &Cand) -> f64 {
    let raw = match c.measured {
        Some(m) => 0.5 * (m.tg.max(0.1) / 10.0).log2(),
        None => SPEED_SPAN - 0.5 * (c.cost / GB / 2.0).max(0.01).log2(),
    };
    raw.clamp(-SPEED_SPAN, SPEED_SPAN)
}

/// Doublings of the context above 8k, at most `cap`.
fn ctx_steps(ctx: u64, cap: f64) -> f64 {
    ((ctx.max(1) as f64 / pick::BASE_CTX as f64).log2()).clamp(0.0, cap)
}

/// Light for a weak PC: all on the graphics card; without one at most [`LIGHT_RAM_SHARE`] of
/// what the RAM may hold.
fn light(c: &Cand, pc: &Pc) -> bool {
    if pc.gpu_on() {
        c.place.fit == Fit::Fits
    } else {
        c.place.ram_bytes as f64 <= pc.ram_budget() as f64 * LIGHT_RAM_SHARE
    }
}

/// `any_light`: a light model fits this PC (else nothing is called heavy).
fn score(c: &Cand, pc: &Pc, needs: &[Need], any_light: bool) -> (f64, Vec<Why>) {
    let has = |n: Need| needs.contains(&n);
    let m = c.m;
    let (rp, de) = (f64::from(m.rp), f64::from(m.de));
    let gpu_on = pc.gpu_on();
    let ctx = c.ctx_rank(gpu_on);
    let mut why: Vec<Why> = Vec::new();
    let mut add = |code: &'static str, value: Option<f64>, points: f64| {
        if points.abs() > 1e-9 {
            why.push(Why { code, value, points });
        }
    };

    // English only: the German part of the quality does not count.
    if has(Need::English) && !has(Need::German) {
        add(if rp >= c.q { "english" } else { "weak_roleplay" }, Some(rp), rp - c.q);
    }
    if has(Need::German) {
        add(if de >= 3.0 { "german" } else { "weak_german" }, Some(de), (de - 3.0) * GERMAN_STEP);
    }
    if has(Need::Quality) {
        add(if rp >= 3.0 { "roleplay" } else { "weak_roleplay" }, Some(rp), (rp - 3.0) * QUALITY_STEP);
    }
    if has(Need::LongReplies) {
        add(if rp >= 3.0 { "roleplay" } else { "weak_roleplay" }, Some(rp), (rp - 3.0) * REPLY_RP_STEP);
    }
    if has(Need::Story) {
        let made_for = m.focus.iter().find(|f| *f == "adventure").or_else(|| m.focus.iter().find(|f| *f == "story"));
        match made_for.map(String::as_str) {
            Some("adventure") => add("adventure", None, STORY_BONUS),
            Some(_) => add("story", None, STORY_BONUS),
            None => {}
        }
    }
    if has(Need::Group) {
        add(if rp >= 3.0 { "roleplay" } else { "weak_roleplay" }, Some(rp), (rp - 3.0) * GROUP_RP_STEP);
        let steps = ctx_steps(ctx, 2.0);
        if steps > 0.0 {
            // The same reason as for long chats and much context: one line in the UI, the points add up.
            add("context", Some(ctx as f64), steps * GROUP_CTX_STEP);
        } else {
            add("little_context", Some(ctx as f64), -GROUP_CTX_STEP);
        }
    }
    if has(Need::LongChats) {
        let steps = ctx_steps(ctx, 3.0);
        if steps > 0.0 {
            add("context", Some(ctx as f64), steps * LONG_CHAT_STEP);
        } else {
            add("little_context", Some(ctx as f64), -LONG_CHAT_STEP);
        }
    }
    if has(Need::Context) {
        let steps = ctx_steps(ctx, 4.0);
        if steps > 0.0 {
            add("context", Some(ctx as f64), steps * CONTEXT_STEP);
        } else {
            add("little_context", Some(ctx as f64), -CONTEXT_STEP);
        }
    }
    let speed = speed_term(c);
    let speed_share = if has(Need::Fast) { FAST_WEIGHT } else if has(Need::WeakPc) { WEAK_SPEED_SHARE } else { 0.0 };
    if speed_share > 0.0 {
        let active = if m.moe { m.params_active_b } else { m.params_total_b };
        let (code, value) = match c.measured {
            Some(t) => (if speed >= 0.0 { "measured_fast" } else { "measured_slow" }, Some(t.tg)),
            None if speed < 0.0 => ("slow", active),
            None if m.moe => ("fast_moe", active),
            None => ("fast_small", active),
        };
        add(code, value, speed * speed_share);
    }
    if has(Need::WeakPc) {
        if light(c, pc) {
            add("light", Some(gb1(if gpu_on { c.place.gpu_bytes } else { c.place.ram_bytes })), LIGHT_BONUS);
        } else if any_light {
            add("heavy", Some(gb1(c.place.gpu_bytes + c.place.ram_bytes)), -HEAVY_COST);
        }
    }
    if has(Need::Uncensored) && m.uncensored == 2 {
        add("uncensored", None, FREE_BONUS);
    }
    if gpu_on && c.place.fit != Fit::Fits {
        add("partly_in_ram", Some(gb1(c.place.ram_bytes)), -if has(Need::Fast) { RAM_HELP_COST_FAST } else { RAM_HELP_COST });
    }
    if c.installed {
        add("installed", None, pick::INSTALLED_BONUS);
    }
    (c.q + why.iter().map(|w| w.points).sum::<f64>(), why)
}

/// Every catalog model that runs on `pc`, the best for `needs` first. "Uncensored" leaves out the
/// models that refuse (`uncensored` 0). Ties: installed, measured, the app's quality, the id.
pub fn suggest(models: &[Sized], pc: &Pc, needs: &[Need]) -> Vec<Suggestion> {
    let open_only = needs.contains(&Need::Uncensored);
    let cands: Vec<Cand> = models.iter().filter(|s| !open_only || s.model.uncensored >= 1).filter_map(|s| candidate(s, pc)).collect();
    let any_light = cands.iter().any(|c| light(c, pc));
    let mut scored: Vec<(f64, Cand, Vec<Why>)> = cands
        .into_iter()
        .map(|c| {
            let (total, why) = score(&c, pc, needs, any_light);
            (total, c, why)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0).then(b.1.installed.cmp(&a.1.installed)).then(b.1.measured.is_some().cmp(&a.1.measured.is_some())).then(b.1.q.total_cmp(&a.1.q)).then_with(|| a.1.m.id.cmp(&b.1.m.id))
    });
    scored
        .into_iter()
        .map(|(total, c, mut why)| {
            why.sort_by(|a, b| b.points.total_cmp(&a.points));
            // The same reason from two needs (roleplay for "group" and "quality") counts once.
            let mut seen: HashSet<&'static str> = HashSet::new();
            let mut merged: Vec<Why> = Vec::new();
            for w in why {
                if seen.insert(w.code) {
                    merged.push(w);
                } else if let Some(first) = merged.iter_mut().find(|x| x.code == w.code) {
                    first.points += w.points;
                }
            }
            let p = &c.place;
            Suggestion {
                id: c.m.id.clone(),
                tier: NAMES[c.tier],
                score: (total * 100.0).round() / 100.0,
                q: (c.q * 10.0).round() / 10.0,
                installed: c.installed,
                measured: c.measured,
                fit: p.fit,
                n_cpu_moe: p.n_cpu_moe,
                gpu_gb: gb1(p.gpu_bytes),
                ram_gb: gb1(p.ram_bytes),
                est_gb: gb1(if p.gpu_bytes > 0 { p.gpu_bytes } else { p.ram_bytes }),
                ctx_max: c.ctx_max,
                reasons: merged.iter().filter(|w| w.points > 0.0).take(3).cloned().collect(),
                caveats: merged.iter().rev().filter(|w| w.points < 0.0).take(2).cloned().collect(),
            }
        })
        .collect()
}

/// The answer shown first: the best [`TOP`] of `ranked`, a base line (`shape`) twice only when no
/// model of another line is within [`TOP_SLACK`] of it (fine-tunes of one base write and run alike).
pub fn top<'a>(ranked: &'a [Suggestion], models: &[Sized]) -> Vec<&'a Suggestion> {
    let shape = |id: &str| models.iter().find(|s| s.model.id == id).map(|s| s.model.shape.as_str()).unwrap_or("");
    let mut out: Vec<&Suggestion> = Vec::new();
    let mut lines: HashSet<&str> = HashSet::new();
    let mut taken: HashSet<&str> = HashSet::new();
    while out.len() < TOP {
        let Some(plain) = ranked.iter().find(|s| !taken.contains(s.id.as_str())) else { break };
        let fresh = ranked.iter().find(|s| !taken.contains(s.id.as_str()) && !lines.contains(shape(&s.id)) && s.score >= plain.score - TOP_SLACK && (s.installed || !plain.installed));
        let s = fresh.unwrap_or(plain);
        taken.insert(s.id.as_str());
        lines.insert(shape(&s.id));
        out.push(s);
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{embedded, Catalog};
    use super::*;
    use rp_core::purpose::{parse, Need::*};
    use rp_core::vram;

    fn sized_all(c: &Catalog) -> Vec<Sized> {
        c.models
            .iter()
            .map(|m| Sized { model: m.clone(), shape: c.shapes[&m.shape].clone(), sizes: m.quants.tiers().map(|(_, q)| q.map(|q| q.size_bytes)), installed: [false; 3], measured: [None; 3] })
            .collect()
    }

    /// CPU PCs with 16 and 32 GB, and 6 to 24 GB of VRAM on every vendor.
    fn profiles() -> Vec<Pc> {
        let mut out = vec![Pc::profile(0, 16, None, "cpu"), Pc::profile(0, 32, None, "cpu")];
        for (vendor, backend) in [("nvidia", "cuda"), ("amd", "rocm"), ("intel", "vulkan")] {
            for vram in [6, 8, 12, 16, 24] {
                out.push(Pc::profile(vram, 32, Some(vendor), backend));
            }
        }
        out
    }

    fn label(p: &Pc) -> String {
        format!("{} GB VRAM / {} GB RAM {}", p.vram_bytes >> 30, p.ram_bytes >> 30, p.backend.as_deref().unwrap_or("-"))
    }

    const SENTENCES: [&str; 6] = [
        "Ich will richtig lange Chats mit 3–4 User-Charakteren",
        "Schnelle Antworten auf Deutsch",
        "Ein unzensiertes Fantasy-Abenteuer erzählen lassen",
        "Viel Kontext für mein Lorebook, beste Qualität",
        "fast replies in English on a weak PC",
        "",
    ];

    /// Six purposes on seventeen simulated PCs: every answer fits the PC, follows its needs and
    /// says why; the purposes lead to different models.
    #[test]
    fn purposes_on_every_pc() {
        let c = embedded();
        let all = sized_all(&c);
        let model = |id: &str| &all.iter().find(|s| s.model.id == id).unwrap().model;
        let mut three = 0;
        for p in profiles() {
            let mut firsts: Vec<String> = Vec::new();
            let mut shown: HashSet<String> = HashSet::new();
            let plain = suggest(&all, &p, &[]);
            assert!(plain.len() >= 4, "{}: {} models fit", label(&p), plain.len());
            for text in SENTENCES {
                let needs = parse(text).needs;
                let ranked = suggest(&all, &p, &needs);
                let picks = top(&ranked, &all);
                let line = picks.iter().map(|s| format!("{} {} {:.2} [{}]", s.id, &s.tier[..1], s.score, s.reasons.iter().map(|w| w.code).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(" | ");
                eprintln!("{} · {text:?}: {line}", label(&p));
                assert_eq!(picks.len(), TOP.min(ranked.len()), "{}", label(&p));
                assert!(ranked.windows(2).all(|w| w[0].score >= w[1].score), "{}: ordered by score", label(&p));
                let first = picks[0];
                firsts.push(first.id.clone());
                shown.extend(picks.iter().map(|s| s.id.clone()));
                for s in &picks {
                    let m = model(&s.id);
                    // It runs the way the suggestion says: the VRAM part and the RAM part fit.
                    let sz = all.iter().find(|x| x.model.id == s.id).unwrap();
                    let tier = NAMES.iter().position(|t| *t == s.tier).unwrap();
                    let info = sz.info(tier).unwrap();
                    let place = pick::place(&info, &p, pick::BASE_CTX);
                    assert_eq!(place.fit, s.fit, "{}: {}", label(&p), s.id);
                    assert!(place.gpu_bytes <= p.vram_budget() && (place.ram_bytes == 0 || p.ram_fits(place.ram_bytes)), "{}: {} does not fit", label(&p), s.id);
                    assert!(vram::estimate(&info, &vram::Load { ctx: s.ctx_max, cache_k: pick::KV.into(), cache_v: pick::KV.into(), ..Default::default() }).is_some());
                    assert!(p.gpu_on() || (s.fit == Fit::Cpu && s.gpu_gb == 0.0));
                    assert!(p.gpu_on() || !m.moe || s.n_cpu_moe == 0);
                    if needs.contains(&Uncensored) {
                        assert!(m.uncensored >= 1, "{}: {} refuses", label(&p), s.id);
                    }
                    // A purpose always comes with a reason; reasons are the terms that helped.
                    assert!(needs.is_empty() || !s.reasons.is_empty() || !s.caveats.is_empty() || s.score == s.q, "{}: {} without a reason", label(&p), s.id);
                    assert!(s.reasons.iter().all(|w| w.points > 0.0) && s.caveats.iter().all(|w| w.points < 0.0));
                    assert!(s.reasons.len() <= 3 && s.caveats.len() <= 2);
                }
                let first_m = model(&first.id);
                let best = |f: &dyn Fn(&Suggestion) -> f64| ranked.iter().map(f).fold(f64::MIN, f64::max);
                if needs.contains(&German) {
                    let top_de = best(&|s| f64::from(model(&s.id).de));
                    // Speed is asked for as well: the best German that is fast enough is among the first three.
                    assert!(picks.iter().take(3).any(|s| f64::from(model(&s.id).de) >= top_de.min(4.0)), "{}: {text}: German {top_de} is missing: {line}", label(&p));
                    assert!(first_m.de >= 2, "{}", first.id);
                }
                if needs.contains(&Story) && ranked.iter().any(|s| model(&s.id).focus.iter().any(|f| f == "story" || f == "adventure")) {
                    assert!(picks.iter().take(2).any(|s| model(&s.id).focus.iter().any(|f| f == "story" || f == "adventure")), "{}: {text}: no story model among the first two: {line}", label(&p));
                }
                if needs.contains(&Context) || needs.contains(&LongChats) {
                    let plain_first = &top(&plain, &all)[0];
                    assert!(first.ctx_max >= plain_first.ctx_max.min(32_768), "{}: {text}: {} holds {} < {}", label(&p), first.id, first.ctx_max, plain_first.ctx_max);
                }
                if needs.contains(&Fast) {
                    let plain_first = &top(&plain, &all)[0];
                    let cost = |id: &str| candidate(all.iter().find(|x| x.model.id == id).unwrap(), &p).unwrap().cost;
                    assert!(cost(&first.id) <= cost(&plain_first.id), "{}: {text}: {} is slower than {}", label(&p), first.id, plain_first.id);
                    assert!(p.gpu_on() && first.fit == Fit::Fits || !p.gpu_on() || first.caveats.iter().any(|w| w.code == "partly_in_ram"), "{}", label(&p));
                }
            }
            let distinct: HashSet<&String> = firsts.iter().collect();
            eprintln!("FIRSTS {}: {} {firsts:?}", label(&p), distinct.len());
            // The purposes change the answer: at least two first places everywhere (on a PC where
            // one MoE model with its experts in RAM is far better than all that fit on the card,
            // it stays first for most purposes), and many models among the first five.
            assert!(distinct.len() >= 2, "{}: the purposes lead to {} first picks: {firsts:?}", label(&p), distinct.len());
            three += usize::from(distinct.len() >= 3);
            assert!(shown.len() >= 9.min(plain.len()), "{}: {} models in the answers", label(&p), shown.len());
        }
        assert!(three >= 9, "three or more first picks on {three} of 17 PCs");
    }

    /// The owner's sentence on his PC (24 GB, ROCm): a model that holds the long chat entirely on
    /// the card, with the context and the roleplay rating as reasons; a dense 24B that only holds
    /// 32k stands behind the MoE models with 128k.
    #[test]
    fn long_group_chats_on_24_gb() {
        let c = embedded();
        let all = sized_all(&c);
        let p = Pc::profile(24, 32, Some("amd"), "rocm");
        let needs = parse("Ich will richtig lange Chats mit 3–4 User-Charakteren").needs;
        assert_eq!(needs, vec![LongChats, Group]);
        let ranked = suggest(&all, &p, &needs);
        let first = &ranked[0];
        assert_eq!((first.id.as_str(), first.fit), ("orion-26b-a4b-v1-1", Fit::Fits), "{:?}", &ranked[..3]);
        assert!(first.ctx_max >= 65_536);
        let codes: Vec<&str> = first.reasons.iter().map(|w| w.code).collect();
        assert_eq!(codes, ["context", "roleplay"], "the context counts once, for long chats and for the group");
        assert!((first.reasons[0].points - (3.0 * LONG_CHAT_STEP + 2.0 * GROUP_CTX_STEP)).abs() < 1e-9);
        // Installed and measured wins among near equals, with the measured context.
        let mut mine = all.clone();
        let i = mine.iter().position(|s| s.model.id == "gemma-4-26b-a4b-heretic").unwrap();
        mine[i].installed[1] = true;
        mine[i].measured[1] = Some(Measured { tg: 60.0, ctx: 131_072 });
        let with_file = suggest(&mine, &p, &needs);
        let place = |list: &[Suggestion], id: &str| list.iter().position(|s| s.id == id).unwrap();
        assert!(place(&with_file, "gemma-4-26b-a4b-heretic") < place(&ranked, "gemma-4-26b-a4b-heretic"));
        assert!(place(&with_file, "gemma-4-26b-a4b-heretic") < place(&with_file, "gemma-4-26b-a4b-sompoa-heresy"), "ahead of its twin that is not on disk");
        let mine_first = &with_file[place(&with_file, "gemma-4-26b-a4b-heretic")];
        assert!(mine_first.reasons.iter().any(|w| w.code == "installed") && mine_first.measured.is_some());
        // The answer does not list five fine-tunes of one base model.
        let ranked = suggest(&all, &p, &needs);
        let picks = top(&ranked, &all);
        let lines: HashSet<&str> = picks.iter().map(|s| all.iter().find(|x| x.model.id == s.id).unwrap().model.shape.as_str()).collect();
        assert!(lines.len() >= 4, "{lines:?}");
    }

    #[test]
    fn terms() {
        let c = embedded();
        let all = sized_all(&c);
        let p = Pc::profile(16, 32, Some("nvidia"), "cuda");
        let by = |needs: &[Need]| suggest(&all, &p, needs);
        let of = |list: &[Suggestion], id: &str| list.iter().find(|s| s.id == id).unwrap().clone();
        // Without needs the score is the app's quality (less what a part in RAM costs).
        let plain = by(&[]);
        assert!(plain.iter().all(|s| s.score == s.q || s.caveats.iter().any(|w| w.code == "partly_in_ram")));
        // German: a model with German 5 gains two steps, one with 2 loses one and says so.
        let german = by(&[German]);
        let m = |id: &str| all.iter().find(|s| s.model.id == id).unwrap().model.clone();
        for s in &german {
            let de = f64::from(m(&s.id).de);
            let got = s.reasons.iter().chain(&s.caveats).find(|w| w.code == "german" || w.code == "weak_german").map_or(0.0, |w| w.points);
            assert!((got - (de - 3.0) * GERMAN_STEP).abs() < 1e-9, "{}", s.id);
        }
        // English: only the roleplay rating counts.
        let english = by(&[English]);
        assert!(english.iter().all(|s| ((s.score - f64::from(m(&s.id).rp)).abs() < 0.011) || !s.caveats.is_empty()), "{:?}", english.first());
        // Uncensored leaves out what refuses; the list is shorter.
        assert!(by(&[Uncensored]).len() < plain.len() && by(&[Uncensored]).iter().all(|s| m(&s.id).uncensored >= 1));
        // Fast: a MoE model all on the card gets the reason, a dense 24B none or the caveat.
        let fast = by(&[Fast]);
        assert_eq!(of(&fast, "gemma-4-26b-a4b-heretic").reasons[0].code, "fast_moe");
        assert!(of(&fast, "cydonia-24b-v4-3").score - of(&plain, "cydonia-24b-v4-3").score < of(&fast, "gemma-4-12b-heretic").score - of(&plain, "gemma-4-12b-heretic").score);
        // Story: models made for adventures move up by the bonus.
        let story = by(&[Story]);
        assert!((of(&story, "wayfarer-2-12b").score - of(&plain, "wayfarer-2-12b").score - STORY_BONUS).abs() < 0.011);
        assert_eq!(of(&story, "wayfarer-2-12b").reasons[0].code, "adventure");
        assert_eq!(of(&story, "gemma-4-12b-it").score, of(&plain, "gemma-4-12b-it").score);
    }
}
