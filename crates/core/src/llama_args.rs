//! What a llama-server build understands: the flags in its `--help`, its version, and the
//! start arguments filtered down to the flags it knows (flags change between builds; an unknown
//! one makes llama-server refuse to start).

use std::collections::HashSet;

/// `-c`, `--ctx-size`: a dash, then a letter (or a second dash and a letter). `-1`, `-`, `-.5`
/// and `--` are values or prose, not flags.
pub fn is_flag(token: &str) -> bool {
    let b = token.as_bytes();
    match b {
        [b'-', b'-', c, ..] => c.is_ascii_alphabetic(),
        [b'-', c, ..] => c.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Every flag that a `--help` text lists. A line that starts with a flag names it and its
/// aliases, separated by commas (`-c,    --ctx-size N`, `--mmap, --no-mmap`); the first token
/// after a flag without a trailing comma is its value or the description.
pub fn parse_help(text: &str) -> HashSet<String> {
    let mut flags = HashSet::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("---") {
            continue; // section headers like `----- common params -----`
        }
        for token in line.split_whitespace() {
            let name = token.trim_end_matches(',');
            if !is_flag(name) {
                break;
            }
            flags.insert(name.to_string());
            if !token.ends_with(',') {
                break;
            }
        }
    }
    flags
}

/// The build number from `--version`: `version: 12387 (1a2b3c4)` or, since the builds carry a
/// semantic version, `version: 1.0.0-dev (build 12387, commit 1a2b3c4)` → `b12387`.
pub fn parse_version(text: &str) -> Option<String> {
    let digits = |s: &str| -> String { s.chars().take_while(char::is_ascii_digit).collect() };
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("version:")?.trim();
        let number = match rest.find("(build ") {
            Some(i) => digits(&rest[i + "(build ".len()..]),
            None => digits(rest),
        };
        (!number.is_empty()).then(|| format!("b{number}"))
    })
}

/// Start arguments with only the flags `known` contains; a dropped flag takes its values
/// (`-x 1 2`) with it. Values before the first flag stay.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Filtered {
    pub args: Vec<String>,
    /// The flags that were left out (names only, without their values).
    pub dropped: Vec<String>,
}

pub fn filter_args(args: &[String], known: &HashSet<String>) -> Filtered {
    let mut out = Filtered::default();
    let mut keep = true;
    for token in args {
        if is_flag(token) {
            let name = token.split('=').next().unwrap_or(token);
            keep = known.contains(name);
            if !keep {
                out.dropped.push(name.to_string());
            }
        }
        if keep {
            out.args.push(token.clone());
        }
    }
    out
}

/// The start options a profile may carry: tuning (threads, context, batches, KV cache, GPU/CPU
/// placement, RoPE), sampler defaults, reasoning, logging and speculative decoding — each with every
/// spelling of current and older builds. Left out on purpose, so a profile (which a phone could set)
/// can neither write files, fetch from the network, open sockets nor change the server's API:
/// `--log-file`, `--slot-save-path`, `--hf-*`/`-hfr`/`-mu`/`--model-url`/`-dr`/`-mmu`, presets that
/// download (`--*-default`), `--rpc`, `--connect`, `--path`, `--api-key*`, `--ssl-*`, `--tools`,
/// `--mcp-*`, `--agent`, `--media-*`, `--models-*`, `--lookup-cache-*`, `--grammar-file`,
/// `--json-schema-file`, `--lora*`, `--control-vector*`, `--embedding`, `--rerank`. Also left out:
/// everything that changes how the chat template renders the prompt (`--chat-template`,
/// `--chat-template-file`, `--chat-template-kwargs`, `--no-jinja`, `--override-kv`, which can replace
/// `tokenizer.chat_template`, `--prefill-assistant`): the fixed rule in the system message must
/// always reach the model (project rule 9).
const ALLOWED: &[&str] = &[
    // threads and CPU placement
    "-t", "--threads", "-tb", "--threads-batch", "-C", "--cpu-mask", "-Cr", "--cpu-range", "--cpu-strict", "--prio", "--poll",
    "-Cb", "--cpu-mask-batch", "-Crb", "--cpu-range-batch", "--cpu-strict-batch", "--prio-batch", "--poll-batch", "--numa",
    // context, batches, slots
    "-c", "--ctx-size", "-n", "--predict", "--n-predict", "-b", "--batch-size", "-ub", "--ubatch-size", "--keep", "--swa-full",
    "-fa", "--flash-attn", "-np", "--parallel", "-cb", "--cont-batching", "-nocb", "--no-cont-batching",
    // KV cache and prompt cache
    "-ctk", "--cache-type-k", "-ctv", "--cache-type-v", "-kvo", "--kv-offload", "-nkvo", "--no-kv-offload", "--cache-prompt",
    "--no-cache-prompt", "--cache-reuse", "-cram", "--cache-ram", "-ctxcp", "--ctx-checkpoints", "--swa-checkpoints", "-cms",
    "--checkpoint-min-step", "-kvu", "--kv-unified", "-no-kvu", "--no-kv-unified", "--kv-unified-per-slot", "--cache-idle-slots",
    "--no-cache-idle-slots", "--context-shift", "--no-context-shift", "-sps", "--slot-prompt-similarity",
    // memory and where the weights go
    "--mmap", "--no-mmap", "--mlock", "-lm", "--load-mode", "-lzm", "--lazy-mode", "--repack", "-nr", "--no-repack", "--no-host",
    "-dev", "--device", "-ot", "--override-tensor", "-cmoe", "--cpu-moe", "-ncmoe", "--n-cpu-moe", "--moe-cache-mib",
    "--moe-cache-cpu-share", "-ncffn", "--n-cpu-ffn", "-ngl", "--gpu-layers", "--n-gpu-layers", "-sm", "--split-mode", "-ts",
    "--tensor-split", "-mg", "--main-gpu", "-fit", "--fit", "-fitt", "--fit-target", "-fitc", "--fit-ctx", "--op-offload",
    "--no-op-offload", "--check-tensors", "--warmup", "--no-warmup",
    // RoPE / YaRN
    "--rope-scaling", "--rope-scale", "--rope-freq-base", "--rope-freq-scale", "--yarn-orig-ctx", "--yarn-ext-factor",
    "--yarn-attn-factor", "--yarn-beta-slow", "--yarn-beta-fast",
    // sampler defaults
    "--samplers", "-s", "--seed", "--sampler-seq", "--sampling-seq", "--ignore-eos", "--temp", "--temperature", "--top-k", "--top-p",
    "--min-p", "--top-nsigma", "--top-n-sigma", "--xtc-probability", "--xtc-threshold", "--typical", "--typical-p", "--repeat-last-n",
    "--repeat-penalty", "--presence-penalty", "--frequency-penalty", "--dry-multiplier", "--dry-base", "--dry-allowed-length",
    "--dry-penalty-last-n", "--dry-sequence-breaker", "--adaptive-target", "--adaptive-decay", "--dynatemp-range", "--dynatemp-exp",
    "--mirostat", "--mirostat-lr", "--mirostat-ent", "-l", "--logit-bias", "-bs", "--backend-sampling",
    // thinking (the model's own template, never another one)
    "--jinja", "--reasoning-format", "-rea", "--reasoning", "--reasoning-effort", "--reasoning-budget", "--reasoning-preserve",
    "--no-reasoning-preserve",
    // logging to the console (the app keeps it), timeouts
    "-v", "--verbose", "--log-verbose", "-lv", "--verbosity", "--log-verbosity", "--log-timestamps", "--no-log-timestamps",
    "--log-prefix", "--no-log-prefix", "--log-colors", "--perf", "--no-perf", "-to", "--timeout", "--threads-http",
    // pictures (the projector file itself: `PATH_FLAGS`)
    "--no-mmproj", "--mmproj-auto", "--no-mmproj-auto", "--mmproj-offload", "--no-mmproj-offload", "-mmdev", "--mmproj-device",
    "--image-min-tokens", "--image-max-tokens",
    // speculative decoding (the draft model file: `PATH_FLAGS`)
    "-ngld", "--gpu-layers-draft", "--n-gpu-layers-draft", "--spec-draft-ngl", "-ctkd", "--cache-type-k-draft", "--spec-draft-type-k",
    "-ctvd", "--cache-type-v-draft", "--spec-draft-type-v", "-td", "--threads-draft", "--spec-draft-threads", "-tbd",
    "--threads-batch-draft", "--spec-draft-threads-batch", "-devd", "--device-draft", "--spec-draft-device", "--draft", "--draft-n",
    "--draft-max", "--spec-draft-n-max", "--draft-min", "--draft-n-min", "--spec-draft-n-min", "--draft-p-min", "--spec-draft-p-min",
    "--spec-type", "-cmoed", "--cpu-moe-draft", "--spec-draft-cpu-moe", "-ncmoed", "--n-cpu-moe-draft", "--spec-draft-n-cpu-moe",
    "--spec-draft-ncmoe", "-otd", "--override-tensor-draft", "--spec-draft-override-tensor",
];

/// Start options that name one model file to read (a projector for pictures, a draft model): allowed
/// with exactly one `.gguf` path on this PC — never a network share (`\\host\…`, which would make
/// Windows log in there), a device path (`\\?\`, `\\.\`) or an address.
const PATH_FLAGS: &[&str] = &["-mm", "--mmproj", "-md", "--model-draft", "--spec-draft-model"];

/// What `dropped` names for values that stand before any flag (never the value itself: it may be a path).
pub const STRAY_VALUE: &str = "<Wert>";

fn local_gguf(value: &str) -> bool {
    let v = value.trim();
    !v.is_empty()
        && v.chars().count() <= 1024
        && !v.contains('\0')
        && !v.contains("://")
        && !v.starts_with("\\\\")
        && !v.starts_with("//")
        && v.to_ascii_lowercase().ends_with(".gguf")
}

/// A flag a profile may carry (with any value; `PATH_FLAGS` see [`allowed_args`]).
pub fn allowed(name: &str) -> bool {
    ALLOWED.contains(&name) || PATH_FLAGS.contains(&name)
}

/// The start arguments with only the options a profile may carry ([`ALLOWED`], [`PATH_FLAGS`] with
/// one local `.gguf` file); a refused flag takes its values (`-x 1 2`, `--x=1`) with it, values before
/// the first flag are refused too ([`STRAY_VALUE`]). The same check guards saving a profile (nothing
/// may be dropped) and every start of the model server (profiles saved before keep working without
/// what is refused now).
pub fn allowed_args(args: &[String]) -> Filtered {
    let mut out = Filtered::default();
    let mut i = 0;
    while i < args.len() {
        let mut end = i + 1;
        while end < args.len() && !is_flag(&args[end]) {
            end += 1;
        }
        let token = &args[i];
        if !is_flag(token) {
            out.dropped.push(STRAY_VALUE.into());
        } else {
            let (name, inline) = match token.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (token.as_str(), None),
            };
            let ok = if PATH_FLAGS.contains(&name) {
                let values: Vec<&str> = inline.into_iter().chain(args[i + 1..end].iter().map(String::as_str)).collect();
                values.len() == 1 && local_gguf(values[0])
            } else {
                ALLOWED.contains(&name)
            };
            if ok {
                out.args.extend_from_slice(&args[i..end]);
            } else {
                out.dropped.push(name.to_string());
            }
        }
        i = end;
    }
    out
}

/// The context size the arguments ask for (`-c N`, `--ctx-size N`, `--ctx-size=N`); `None` when
/// they leave it out, or set 0 (= the model's training context).
pub fn ctx_from_args(args: &[String]) -> Option<u64> {
    let mut found = None;
    let mut it = args.iter();
    while let Some(token) = it.next() {
        let value = match token.split_once('=') {
            Some(("-c" | "--ctx-size", v)) => Some(v.to_string()),
            None if token == "-c" || token == "--ctx-size" => it.next().cloned(),
            _ => None,
        };
        if let Some(v) = value {
            found = v.trim().parse::<u64>().ok(); // the last one wins, like in llama-server
        }
    }
    found.filter(|&n| n > 0)
}

/// The arguments without the flags `names` (and their values, `-c 4096` and `-c=4096` alike).
pub fn without_flags(args: &[String], names: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut skip = false;
    for token in args {
        if is_flag(token) {
            skip = names.contains(&token.split('=').next().unwrap_or(token));
        }
        if !skip {
            out.push(token.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_flags_with_their_values() {
        let a: Vec<String> = ["-ngl", "0", "-c", "4096", "--n-cpu-moe=8", "-np", "1", "--ctx-size", "2048"].iter().map(|s| s.to_string()).collect();
        assert_eq!(without_flags(&a, &["-c", "--ctx-size", "--n-cpu-moe"]), vec!["-ngl", "0", "-np", "1"]);
        assert_eq!(without_flags(&a, &[]), a);
    }

    /// Lines as llama-server b12xxx prints them.
    const HELP: &str = "----- common params -----

-h,    --help, --usage                  print usage and exit
--version                               show version and build info
-t,    --threads N                      number of CPU threads to use during generation
                                        (default: -1)
                                        (env: LLAMA_ARG_THREADS)
-c,    --ctx-size N                     size of the prompt context (default: 4096, 0 = loaded
                                        from model)
-fa,   --flash-attn [on|off|auto]       set Flash Attention use ('on', 'off', or 'auto',
                                        default: 'auto')
--mmap, --no-mmap                       whether to memory-map model
-ngl,  --gpu-layers, --n-gpu-layers N   max. number of layers to store in VRAM
-ctk,  --cache-type-k TYPE              KV cache data type for K
                                        - none: nothing
                                        -1 = all
--slot-save-path PATH                   path to save slot kv cache (default: disabled)
";

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_lists_every_alias() {
        let flags = parse_help(HELP);
        for f in ["-h", "--help", "--usage", "--version", "-t", "--threads", "-c", "--ctx-size", "-fa", "--flash-attn", "--mmap",
            "--no-mmap", "-ngl", "--gpu-layers", "--n-gpu-layers", "-ctk", "--cache-type-k", "--slot-save-path"]
        {
            assert!(flags.contains(f), "{f} missing");
        }
        assert!(!flags.iter().any(|f| f.starts_with("---") || f == "-" || f == "-1" || f == "N"), "{flags:?}");
        assert_eq!(flags.len(), 18);
    }

    #[test]
    fn unknown_flags_and_their_values_are_dropped() {
        let known = parse_help(HELP);
        let args = strings(&["-c", "4096", "-ngl", "999", "--n-cpu-moe", "12", "-fa", "on", "--seed", "-1", "--mmap", "--ctx-size=2048", "--bogus=3"]);
        let f = filter_args(&args, &known);
        assert_eq!(f.args, strings(&["-c", "4096", "-ngl", "999", "-fa", "on", "--mmap", "--ctx-size=2048"]));
        assert_eq!(f.dropped, strings(&["--n-cpu-moe", "--seed", "--bogus"]), "-1 is the value of --seed, not a flag");
        assert_eq!(filter_args(&[], &known), Filtered::default());
    }

    #[test]
    fn allowlist_keeps_tuning_and_sampler_options() {
        let args = strings(&[
            "-c", "8192", "-ngl", "99", "--n-cpu-moe", "12", "-ctk", "q8_0", "--cache-type-v=q8_0", "-fa", "on", "-t", "8", "-b", "2048",
            "-ub", "512", "--mlock", "--no-mmap", "-np", "1", "--temp", "0.8", "--min-p", "0.05", "--dry-multiplier", "0.8", "--jinja",
            "--reasoning-budget", "0", "-lv", "4", "--seed", "-1", "-ot", "blk\\..*exps=CPU", "--no-warmup", "--keep", "64",
            "--cache-reuse", "256", "--mmproj", "C:\\models\\mmproj-x-f16.gguf", "-md", "D:/llm/draft.gguf",
        ]);
        let f = allowed_args(&args);
        assert_eq!((f.args, f.dropped), (args, Vec::<String>::new()));
        for name in ["--ctx-size", "--gpu-layers", "--n-gpu-layers", "-ncmoe", "--flash-attn", "--threads", "--batch-size", "--ubatch-size", "--mmap", "--parallel"] {
            assert!(allowed(name), "{name}");
        }
    }

    #[test]
    fn allowlist_drops_files_network_and_downloads() {
        let args = strings(&[
            "-c", "4096", "--log-file", "C:\\Users\\x\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\x.bat", "-hfr",
            "someone/evil-model", "--rpc", "203.0.113.5:50052", "--slot-save-path", "C:\\x", "-mu", "https://example.invalid/x.gguf",
            "--model-url=https://example.invalid/y.gguf", "--hf-repo", "a/b", "-hf", "a/b", "-hff", "x.gguf", "-hft", "t", "--api-key", "k",
            "--path", "C:\\", "--tools", "all", "--mcp-servers-config", "c.json", "--agent", "--connect", "--media-allow-private-urls",
            "--gpt-oss-20b-default", "--spec-default", "-mmu", "https://x", "--lora", "x.gguf", "--grammar-file", "g", "-ngl", "0",
        ]);
        let f = allowed_args(&args);
        assert_eq!(f.args, strings(&["-c", "4096", "-ngl", "0"]), "only the tuning stays, the values go with their flags");
        assert_eq!(f.dropped.len(), 22, "{:?}", f.dropped);
        assert!(f.dropped.iter().all(|d| d.starts_with('-')), "{:?}", f.dropped);
        // Model files from the network, a share or a device path; more than one file; none.
        for bad in [
            &["--mmproj", "\\\\attacker\\share\\x.gguf"][..],
            &["-mm", "//attacker/share/x.gguf"],
            &["--mmproj=\\\\?\\C:\\x.gguf"],
            &["-md", "https://example.invalid/x.gguf"],
            &["-md", "C:\\x.bin"],
            &["-md", "a.gguf", "b.gguf"],
            &["--model-draft"],
        ] {
            assert_eq!(allowed_args(&strings(bad)).args, Vec::<String>::new(), "{bad:?}");
        }
        // Values before any flag never pass, and are not named (they may be paths).
        let f = allowed_args(&strings(&["C:\\secret\\x.gguf", "-c", "2048"]));
        assert_eq!((f.args, f.dropped), (strings(&["-c", "2048"]), vec![STRAY_VALUE.to_string()]));
    }

    /// Project rule 9: no start option may change how the chat template renders the system message
    /// (the fixed rule lives there).
    #[test]
    fn allowlist_keeps_the_model_template() {
        for flag in ["--chat-template", "--chat-template-file", "--chat-template-kwargs", "--no-jinja", "--override-kv", "--prefill-assistant", "--skip-chat-parsing", "--reasoning-budget-message"] {
            assert!(!allowed(flag), "{flag}");
            let f = allowed_args(&strings(&[flag, "{% for m in messages %}{% if m.role != 'system' %}{{ m.content }}{% endif %}{% endfor %}", "-c", "4096"]));
            assert_eq!((f.args, f.dropped), (strings(&["-c", "4096"]), vec![flag.to_string()]));
            let inline = format!("{flag}=chatml");
            assert_eq!(allowed_args(&strings(&[&inline])).dropped, vec![flag.to_string()]);
        }
        assert_eq!(allowed_args(&strings(&["--override-kv", "tokenizer.chat_template=str:{{ messages[1].content }}"])).args, Vec::<String>::new());
        assert!(allowed("--jinja"), "the model's own template, the default of current builds");
    }

    #[test]
    fn version_and_context() {
        assert_eq!(parse_version("load_backend: loaded CPU\nversion: 12387 (1a2b3c4)\nbuilt with clang"), Some("b12387".into()));
        assert_eq!(parse_version("version: 1.0.0-dev (build 12387, commit a956a852c)
built with Clang"), Some("b12387".into()));
        assert_eq!(parse_version("nothing here"), None);
        assert_eq!(ctx_from_args(&strings(&["-ngl", "0", "-c", "2048"])), Some(2048));
        assert_eq!(ctx_from_args(&strings(&["--ctx-size=8192", "-c", "4096"])), Some(4096), "the last one wins");
        assert_eq!(ctx_from_args(&strings(&["-c", "0"])), None);
        assert_eq!(ctx_from_args(&strings(&["-c"])), None);
        assert_eq!(ctx_from_args(&strings(&["-np", "1"])), None);
    }
}
