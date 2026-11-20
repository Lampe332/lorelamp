//! With the `embed-ui` feature the built web UI is compiled into the program (see `src/web.rs`).
//! Fail early and clearly when it has not been built, and rebuild when it changes.

use std::path::Path;

fn main() {
    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_none() {
        return;
    }
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist");
    // A directory makes Cargo watch every file below it.
    println!("cargo:rerun-if-changed={}", dist.display());
    if !dist.join("index.html").is_file() {
        panic!(
            "\n\nThe web UI is not built: {} has no index.html.\n\
             Run `npm run build` in apps/web first (the feature `embed-ui` puts it into the program).\n",
            dist.display()
        );
    }
}
