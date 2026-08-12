//! Dev tool: dump the resolved role-prose constants and the exact bytes the
//! migrations before 0049 seeded, so `0049_role_prose_drops_the_names.sql` can
//! be written from the compiler's own view of the literals rather than by hand.
//!
//! `cargo run --example dump_role_prose -- <out-dir>`
fn main() {
    let dir = std::env::args().nth(1).expect("usage: dump_role_prose <out-dir>");
    let d = std::path::Path::new(&dir);
    std::fs::write(d.join("hands_new.txt"), bot_hq::agents::prompts::HANDS_ROLE).unwrap();
    std::fs::write(d.join("eyes_new.txt"), bot_hq::agents::prompts::EYES_ROLE).unwrap();
    println!(
        "hands={} eyes={}",
        bot_hq::agents::prompts::HANDS_ROLE.len(),
        bot_hq::agents::prompts::EYES_ROLE.len()
    );
}
