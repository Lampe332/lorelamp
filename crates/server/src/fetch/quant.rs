//! Pure helpers for Hugging Face links and GGUF files: which repo a pasted link means, which
//! quant a file name carries, multi-part files, and the three size tiers the UI offers instead of
//! quant names (Klein / Ausgewogen / Groß).

use rp_models::RepoFile;
use serde::Serialize;

/// The three tiers. `small` < 4 bit (incl. IQ4_XS/IQ4_NL), `balanced` 4 bit, `large` ≥ 5 bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Small,
    Balanced,
    Large,
}

/// Known quant names, smallest first. The index is the rank.
const QUANTS: &[&str] = &[
    "IQ1_S", "IQ1_M", "TQ1_0", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "TQ2_0", "Q2_K_S", "Q2_K", "Q2_K_L", "Q2_K_XL",
    "IQ3_XXS", "IQ3_XS", "IQ3_S", "IQ3_M", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q3_K_XL", "IQ4_XS", "IQ4_NL", "Q4_0", "Q4_1",
    "MXFP4", "Q4_K_S", "Q4_K_M", "Q4_K_L", "Q4_K_XL", "Q5_0", "Q5_1", "Q5_K_S", "Q5_K_M", "Q5_K_L", "Q5_K_XL", "Q6_K",
    "Q6_K_L", "Q6_K_XL", "Q8_0", "Q8_K_XL", "BF16", "F16", "F32",
];

/// Which file of a tier is suggested first.
const PREFER_SMALL: &[&str] = &["IQ4_XS", "Q3_K_L", "Q3_K_XL", "Q3_K_M", "IQ4_NL", "IQ3_M", "Q3_K_S", "IQ3_S", "IQ3_XS"];
const PREFER_BALANCED: &[&str] = &["Q4_K_M", "Q4_K_S", "Q4_K_L", "Q4_K_XL", "Q4_0", "Q4_1", "MXFP4"];
const PREFER_LARGE: &[&str] = &["Q6_K", "Q5_K_M", "Q6_K_L", "Q5_K_L", "Q5_K_S", "Q6_K_XL", "Q5_K_XL", "Q8_0"];

fn rank(q: &str) -> Option<usize> {
    QUANTS.iter().position(|k| *k == q)
}

pub fn tier_of(quant: &str) -> Option<Tier> {
    let r = rank(quant)?;
    Some(if r < rank("Q4_0").unwrap_or(0) {
        Tier::Small
    } else if r < rank("Q5_0").unwrap_or(0) {
        Tier::Balanced
    } else {
        Tier::Large
    })
}

/// The quant named in a file name or path (`…/Model.i1-Q4_K_M.gguf` → `Q4_K_M`). The longest
/// known name that stands between separators wins; `None` when none does.
pub fn quant_of(name: &str) -> Option<&'static str> {
    let up = name.to_ascii_uppercase();
    let b = up.as_bytes();
    let sep = |i: usize| i >= b.len() || !b[i].is_ascii_alphanumeric();
    let mut best: Option<&'static str> = None;
    for q in QUANTS {
        let mut from = 0;
        while let Some(pos) = up[from..].find(q) {
            let at = from + pos;
            let before_ok = at == 0 || !b[at - 1].is_ascii_alphanumeric();
            if before_ok && sep(at + q.len()) && best.is_none_or(|x| q.len() > x.len()) {
                best = Some(q);
            }
            from = at + 1;
        }
    }
    best
}

/// `model-00001-of-00003.gguf` → (`model`, 1, 3).
pub fn split_part(path: &str) -> Option<(&str, u32, u32)> {
    let stem = path.strip_suffix(".gguf").or_else(|| path.strip_suffix(".GGUF"))?;
    let (rest, count) = stem.rsplit_once("-of-")?;
    let (base, idx) = rest.rsplit_once('-')?;
    let digits = |s: &str| s.len() == 5 && s.bytes().all(|c| c.is_ascii_digit());
    if !digits(idx) || !digits(count) {
        return None;
    }
    let (idx, count) = (idx.parse().ok()?, count.parse().ok()?);
    (idx >= 1 && idx <= count).then_some((base, idx, count))
}

pub fn is_gguf(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".gguf")
}

/// A projector for pictures (vision models): `mmproj-…gguf`.
pub fn is_mmproj(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    is_gguf(path) && name.contains("mmproj")
}

/// One downloadable model choice: a single file or all parts of a multi-part file.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Choice {
    /// The file, or the shared name of the parts (without `-0000N-of-0000M.gguf`).
    pub key: String,
    pub quant: Option<&'static str>,
    pub tier: Option<Tier>,
    pub files: Vec<RepoFile>,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TierGroup {
    /// Key of the suggested choice; None for an empty tier.
    pub recommended: Option<String>,
    /// Smallest first.
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Grouped {
    pub small: TierGroup,
    pub balanced: TierGroup,
    pub large: TierGroup,
    /// GGUF files without a known quant name.
    pub other: Vec<Choice>,
    pub mmproj: Vec<RepoFile>,
}

/// Groups a repo's files: multi-part sets become one choice (incomplete sets are left out),
/// each choice lands in its tier, every tier names its suggested choice.
pub fn group(files: &[RepoFile]) -> Grouped {
    let mut out = Grouped::default();
    let mut choices: Vec<Choice> = Vec::new();
    let mut parts: Vec<(String, u32, RepoFile)> = Vec::new();
    for f in files.iter().filter(|f| is_gguf(&f.path)) {
        if is_mmproj(&f.path) {
            out.mmproj.push(f.clone());
            continue;
        }
        match split_part(&f.path) {
            Some((base, _, count)) => parts.push((base.to_string(), count, f.clone())),
            None => choices.push(choice(f.path.clone(), vec![f.clone()])),
        }
    }
    parts.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.path.cmp(&b.2.path)));
    let mut i = 0;
    while i < parts.len() {
        let (base, count) = (parts[i].0.clone(), parts[i].1);
        let set: Vec<RepoFile> = parts[i..].iter().take_while(|p| p.0 == base).map(|p| p.2.clone()).collect();
        i += set.len();
        if set.len() as u32 == count && set.iter().all(|f| split_part(&f.path).is_some_and(|(_, _, c)| c == count)) {
            choices.push(choice(base, set));
        }
    }
    choices.sort_by_key(|c| c.size);
    for c in choices {
        match c.tier {
            Some(Tier::Small) => out.small.choices.push(c),
            Some(Tier::Balanced) => out.balanced.choices.push(c),
            Some(Tier::Large) => out.large.choices.push(c),
            None => out.other.push(c),
        }
    }
    for (g, prefer) in [(&mut out.small, PREFER_SMALL), (&mut out.balanced, PREFER_BALANCED), (&mut out.large, PREFER_LARGE)] {
        g.recommended = prefer
            .iter()
            .find_map(|q| g.choices.iter().find(|c| c.quant == Some(*q)))
            .or(g.choices.first())
            .map(|c| c.key.clone());
    }
    out
}

fn choice(key: String, files: Vec<RepoFile>) -> Choice {
    let quant = quant_of(&key);
    Choice { quant, tier: quant.and_then(tier_of), size: files.iter().map(|f| f.size).sum(), key, files }
}

/// One path segment of a repo id: letters, digits, `.`, `_`, `-`; not starting with `.` or `-`.
fn repo_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && !s.starts_with(['.', '-'])
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// `org/name`.
pub fn valid_repo(repo: &str) -> bool {
    matches!(repo.split_once('/'), Some((a, b)) if repo_segment(a) && repo_segment(b))
}

/// A GGUF path inside a repo: relative, no `..`, no backslash, no drive, ends in `.gguf`; no
/// segment that Windows reads as a device (`NUL.gguf`, `COM1/…`) or changes (a dot or space at the
/// end).
pub fn valid_file(path: &str) -> bool {
    path.len() <= 512
        && is_gguf(path)
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path.split('/').all(|s| !s.is_empty() && s != "." && s != ".." && !s.starts_with(' ') && !s.ends_with([' ', '.']) && !windows_device(s))
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9`, alone or before the first dot.
fn windows_device(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        s => (s.starts_with("COM") || s.starts_with("LPT")) && s.len() == 4 && s.as_bytes()[3].is_ascii_digit(),
    }
}

/// What a pasted link points at: the repo and, for a link to one file, that file.
/// Takes `org/name`, `https://huggingface.co/org/name[/tree|blob|resolve/<rev>/<path>]` and the
/// same with `hf.co`; query and fragment are ignored. Datasets and Spaces are refused.
pub fn parse_link(input: &str) -> Option<(String, Option<String>)> {
    let s = input.trim();
    let s = s.split(['?', '#']).next().unwrap_or("");
    let rest = if let Some(r) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
        let (host, path) = r.split_once('/').unwrap_or((r, ""));
        let host = host.to_ascii_lowercase();
        if !matches!(host.as_str(), "huggingface.co" | "www.huggingface.co" | "hf.co") {
            return None;
        }
        path
    } else if let Some(r) = s.strip_prefix("huggingface.co/").or_else(|| s.strip_prefix("hf.co/")) {
        r
    } else {
        s
    };
    let segs: Vec<&str> = rest.split('/').filter(|x| !x.is_empty()).collect();
    if segs.len() < 2 || matches!(segs[0], "datasets" | "spaces" | "api" | "models" | "collections") {
        return None;
    }
    let repo = format!("{}/{}", segs[0], segs[1]);
    if !valid_repo(&repo) {
        return None;
    }
    let file = match segs.get(2) {
        Some(&("blob" | "resolve")) if segs.len() > 4 => {
            let p = segs[4..].join("/");
            let p = percent_decode(&p);
            valid_file(&p).then_some(p)
        }
        _ => None,
    };
    Some((repo, file))
}

/// `%20` and friends in a pasted file link (UTF-8; a broken escape stays as it is).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Owners whose GGUF repos "Mehr finden" shows (the known quanters; the catalog adds its own).
pub const QUANTERS: &[&str] = &["unsloth", "bartowski", "mradermacher", "lmstudio-community", "ggml-org"];

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, size: u64) -> RepoFile {
        RepoFile { path: path.into(), size, sha256: None }
    }

    #[test]
    fn quant_names() {
        assert_eq!(quant_of("Famino-12B-Model_Stock.i1-Q4_K_M.gguf"), Some("Q4_K_M"));
        assert_eq!(quant_of("gemma-4-31B-it-IQ4_XS.gguf"), Some("IQ4_XS"));
        assert_eq!(quant_of("Model-UD-Q4_K_XL.gguf"), Some("Q4_K_XL"));
        assert_eq!(quant_of("Q5_K_M/Model-00001-of-00002.gguf"), Some("Q5_K_M"));
        assert_eq!(quant_of("model.q6_k.gguf"), Some("Q6_K"));
        assert_eq!(quant_of("model-f16.gguf"), Some("F16"));
        assert_eq!(quant_of("model-BF16.gguf"), Some("BF16"));
        assert_eq!(quant_of("XQ4_K_M.gguf"), None, "no separator before the name");
        assert_eq!(quant_of("tinygemma3.gguf"), None);
        assert_eq!(tier_of("Q3_K_M"), Some(Tier::Small));
        assert_eq!(tier_of("IQ4_XS"), Some(Tier::Small));
        assert_eq!(tier_of("Q4_K_M"), Some(Tier::Balanced));
        assert_eq!(tier_of("Q5_K_M"), Some(Tier::Large));
        assert_eq!(tier_of("F16"), Some(Tier::Large));
    }

    #[test]
    fn parts() {
        assert_eq!(split_part("a/Model-Q8_0-00002-of-00003.gguf"), Some(("a/Model-Q8_0", 2, 3)));
        assert_eq!(split_part("Model-Q8_0.gguf"), None);
        assert_eq!(split_part("Model-00004-of-00003.gguf"), None);
        assert_eq!(split_part("Model-1-of-3.gguf"), None);
    }

    #[test]
    fn grouping() {
        let g = group(&[
            f("README.md", 1),
            f("M-Q3_K_M.gguf", 30),
            f("M-IQ4_XS.gguf", 38),
            f("M-Q4_K_S.gguf", 41),
            f("M-Q4_K_M.gguf", 43),
            f("M-Q6_K.gguf", 60),
            f("Q8_0/M-Q8_0-00001-of-00002.gguf", 40),
            f("Q8_0/M-Q8_0-00002-of-00002.gguf", 41),
            f("F16/M-F16-00001-of-00003.gguf", 50),
            f("mmproj-M-f16.gguf", 5),
            f("M.imatrix.gguf", 1),
        ]);
        assert_eq!(g.small.choices.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["M-Q3_K_M.gguf", "M-IQ4_XS.gguf"]);
        assert_eq!(g.small.recommended.as_deref(), Some("M-IQ4_XS.gguf"));
        assert_eq!(g.balanced.recommended.as_deref(), Some("M-Q4_K_M.gguf"));
        assert_eq!(g.large.choices.iter().map(|c| (c.key.as_str(), c.size)).collect::<Vec<_>>(), [("M-Q6_K.gguf", 60), ("Q8_0/M-Q8_0", 81)]);
        assert_eq!(g.large.recommended.as_deref(), Some("M-Q6_K.gguf"));
        assert_eq!(g.large.choices[1].files.len(), 2, "both parts");
        assert_eq!(g.mmproj.len(), 1);
        assert_eq!(g.other.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["M.imatrix.gguf"], "the incomplete F16 set is left out");
    }

    #[test]
    fn links() {
        let r = |s: &str| parse_link(s);
        assert_eq!(r("bartowski/Model-GGUF"), Some(("bartowski/Model-GGUF".into(), None)));
        assert_eq!(r(" https://huggingface.co/bartowski/Model-GGUF/tree/main "), Some(("bartowski/Model-GGUF".into(), None)));
        assert_eq!(r("https://hf.co/a/b?x=1#y"), Some(("a/b".into(), None)));
        assert_eq!(r("huggingface.co/a/b"), Some(("a/b".into(), None)));
        assert_eq!(
            r("https://huggingface.co/a/b/blob/main/sub/M%20x-Q4_K_M.gguf?download=true"),
            Some(("a/b".into(), Some("sub/M x-Q4_K_M.gguf".into())))
        );
        assert_eq!(r("https://huggingface.co/a/b/resolve/main/M-Q4_K_M.gguf"), Some(("a/b".into(), Some("M-Q4_K_M.gguf".into()))));
        assert_eq!(r("https://huggingface.co/a/b/blob/main/README.md"), Some(("a/b".into(), None)));
        assert_eq!(r("https://huggingface.co/datasets/a/b"), None);
        assert_eq!(r("https://huggingface.co/spaces/a/b"), None);
        assert_eq!(r("https://evil.example/a/b"), None);
        assert_eq!(r("a"), None);
        assert_eq!(r("../b"), None);
        assert_eq!(r("a/b c"), None);
    }

    #[test]
    fn file_paths() {
        assert!(valid_file("M-Q4_K_M.gguf"));
        assert!(valid_file("Q8_0/M-00001-of-00002.gguf"));
        for bad in ["../x.gguf", "/x.gguf", "a\\x.gguf", "C:x.gguf", "a//x.gguf", "x.bin", "./x.gguf"] {
            assert!(!valid_file(bad), "{bad}");
        }
        // Windows device names and a folder name with a dot at the end (Windows drops it).
        for bad in ["NUL.gguf", "com1.gguf", "Lpt9.Q4_K_M.gguf", "con/x.gguf", "aux.x/y.gguf", "Q8_0./x.gguf", "x /y.gguf"] {
            assert!(!valid_file(bad), "{bad}");
        }
        for good in ["NULL.gguf", "COM10.gguf", "console/x.gguf", "Q8_0/x.gguf"] {
            assert!(valid_file(good), "{good}");
        }
        assert!(valid_repo("llmfan46/gemma-4-E4B-it-ultra-uncensored-heretic-GGUF"));
        assert!(!valid_repo("a/b/c"), "only two segments");
        assert!(!valid_repo("a"));
        assert!(!valid_repo(".a/b"));
    }
}
