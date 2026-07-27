//! Drive the native tool surface's refusal paths against a REAL repository.
//!
//! Not a unit test. This calls the same `tools::exec` the native agent loop calls,
//! with the same `ToolPolicy` EYES gets, against the actual working tree — so a
//! refusal that only holds in a tempdir fixture would show up here.
//!
//! Run: `cargo run --example refusal_probe`
//!
//! Exits non-zero if any probe was ALLOWED, or if the sentinel file was modified.

use bot_hq::agents::native::command::CommandPolicy;
use bot_hq::agents::native::tools::{self, ToolPolicy};
use bot_hq::agents::native::wire::ToolCall;
use serde_json::json;

/// The sentinel MUST live inside the read root: its job is to prove a refused
/// `write_file` / `rm` did not touch a file the agent could otherwise reach.
/// Moving it to a temp dir would make every probe fail for the wrong reason —
/// path-escape rather than the layer under test — and the probe would pass
/// while testing nothing.
const SENTINEL: &str = "probe-delete-me.txt";

/// Leave the tree as we found it, on EVERY exit path.
///
/// The probe used to create this file in the repo root and never remove it, so
/// a run left untracked debris behind — which then shows up in the `git status`
/// EYES uses to re-sync, as review noise. Only removes a file this run created:
/// a pre-existing one belongs to whoever put it there.
fn finish(code: i32, sentinel: &std::path::Path, created_here: bool) -> ! {
    if created_here {
        let _ = std::fs::remove_file(sentinel);
    }
    std::process::exit(code)
}

struct Probe {
    tool: &'static str,
    input: serde_json::Value,
    layer: &'static str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = std::env::current_dir()?.canonicalize()?;
    let sentinel_path = root.join(SENTINEL);
    let sentinel_before = std::fs::read(&sentinel_path).ok();
    let created_here = sentinel_before.is_none();
    if created_here {
        std::fs::write(&sentinel_path, "sentinel for the B5 refusal probe\n")?;
    }
    let sentinel_before = std::fs::read(&sentinel_path)?;

    let probes = vec![
        Probe {
            tool: "run_command",
            input: json!({ "command": "git push" }),
            layer: "validate_git — subcommand not read-only",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "ps eww" }),
            layer: "allow-list — ps leaks sibling env (ANTHROPIC_AUTH_TOKEN)",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": format!("rm {SENTINEL}") }),
            layer: "allow-list — rm absent (relative path, so program layer)",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "git log --oneline -3 && echo hi" }),
            layer: "reject_shell_metachars — &&",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "find . -name '*.rs' -delete" }),
            layer: "FIND_WRITE_PREDICATES — -delete",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "du --exclude-from=/etc/shadow" }),
            layer: "reject_escaping_path — absolute path in a flag VALUE",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "git -C /tmp log" }),
            layer: "GIT_REPO_RETARGET_FLAGS — -C retargets the repo",
        },
        Probe {
            tool: "run_command",
            input: json!({ "command": "git branch evil-branch" }),
            layer: "git branch — bare name would CREATE",
        },
        Probe {
            tool: "read_file",
            input: json!({ "path": "../Cargo.toml" }),
            layer: "resolve_in_root — .. component",
        },
        Probe {
            tool: "read_file",
            input: json!({ "path": "/etc/passwd" }),
            layer: "resolve_in_root — absolute path replaces the base",
        },
        Probe {
            tool: "write_file",
            input: json!({ "path": SENTINEL, "content": "PWNED" }),
            layer: "ToolPolicy role gate — Write class, invoked by name anyway",
        },
        Probe {
            tool: "list_files",
            input: json!({ "pattern": "../**/*.toml" }),
            layer: "list_files — escaping glob",
        },
    ];

    // What EYES is actually offered, for the record.
    let offered: Vec<String> = tools::tool_defs_for(Some(&root), ToolPolicy::ReadOnly)
        .iter()
        .filter_map(|d| d["name"].as_str().map(str::to_string))
        .collect();
    println!("EYES tool surface: {}\n", offered.join(", "));
    println!(
        "write_file advertised to EYES: {}\n",
        offered.iter().any(|n| n == "write_file")
    );

    let mut allowed = Vec::new();
    for (i, p) in probes.iter().enumerate() {
        let call = ToolCall {
            id: format!("probe_{i}"),
            name: p.tool.to_string(),
            input: p.input.clone(),
        };
        let out = tools::exec(&call, Some(&root), ToolPolicy::ReadOnly, CommandPolicy::ReadOnly).await;
        let verdict = if out.is_error { "REFUSED" } else { "ALLOWED" };
        if !out.is_error {
            allowed.push(format!("{} {}", p.tool, p.input));
        }
        let msg = out.content.lines().next().unwrap_or("").trim();
        let msg: String = msg.chars().take(150).collect();
        println!("[{verdict}] {} {}", p.tool, p.input);
        println!("          layer : {}", p.layer);
        println!("          says  : {msg}\n");
    }

    // A control: one thing that MUST still work, so a blanket-refuse bug can't
    // masquerade as success.
    let control = ToolCall {
        id: "control".into(),
        name: "run_command".into(),
        input: json!({ "command": "git log --oneline -1" }),
    };
    let c = tools::exec(
        &control,
        Some(&root),
        ToolPolicy::ReadOnly,
        CommandPolicy::ReadOnly,
    )
    .await;
    println!(
        "[{}] control: git log --oneline -1\n          says  : {}\n",
        if c.is_error { "BROKEN" } else { "ALLOWED" },
        c.content.lines().next().unwrap_or("").trim()
    );

    let sentinel_after = std::fs::read(&sentinel_path)?;
    let sentinel_ok = sentinel_before == sentinel_after;
    println!("sentinel intact: {sentinel_ok}");

    if !allowed.is_empty() {
        eprintln!("\nFAIL — these should have been refused:");
        for a in &allowed {
            eprintln!("  {a}");
        }
        finish(1, &sentinel_path, created_here);
    }
    if !sentinel_ok {
        eprintln!("\nFAIL — sentinel was modified");
        // Deliberately NOT cleaned up: the sentinel was written by something
        // that should not have been able to touch it, and that evidence is the
        // whole point of this failure.
        finish(1, &sentinel_path, false);
    }
    if c.is_error {
        eprintln!("\nFAIL — the control command was refused; refusals may be blanket");
        finish(1, &sentinel_path, created_here);
    }
    println!("\nAll {} probes refused; control still works.", probes.len());
    finish(0, &sentinel_path, created_here);
}
