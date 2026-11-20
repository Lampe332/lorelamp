// Thin entry for a desktop run of the same crate; on Android `rp_mobile_lib::run` is the entry.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rp_mobile_lib::run();
}
