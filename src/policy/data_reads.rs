//! Production and staging data reads as project policy (feedback #51 #59 #62
//! #94, group K).
//!
//! The rule "a production read needs the user's approval" lived in prose and
//! one global keyword (`psql -h ep-solitary-field-`), matched anywhere in a
//! line. Six kinds of `gcloud` read matched nothing, and both participants of
//! one session ran them ungated all day until the user said "you have not
//! gated any reads. HARD CHECK" (#51). The user's picks (tray `5660fc1e`,
//! `d9555879`): a per-project `production_reads` list — and `staging_reads` —
//! matched against the command a line actually RUNS, parking every match for
//! approval, for every participant, after the reviewer reads it.
//!
//! An entry is words. The first is the tool and must be the command's tool
//! (its basename). Words starting with `-` are spelling only — `psql -h HOST`
//! must also catch `--host=HOST`, `-hHOST` and a URL. The other words must
//! appear as whole argument words, in order, except the LAST, which may sit
//! anywhere inside any word of the command, including the `NAME=value` words
//! before it (`PGHOST=ep-… psql`) — the host (EYES, s-3158eb35). Over-matching
//! (`gcloud run` against a `--runtime` flag) only parks a command for
//! approval, which is the safe direction.

use super::Policy;

/// Which list matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataReadKind {
    Production,
    Staging,
}

impl DataReadKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
        }
    }
}

/// A command that runs a listed data read: which list, and the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRead {
    pub kind: DataReadKind,
    pub entry: String,
}

/// The first list entry a command `command` RUNS matches — production before
/// staging — or `None`.
pub fn match_command(policy: &Policy, command: &str) -> Option<DataRead> {
    if policy.production_reads.is_empty() && policy.staging_reads.is_empty() {
        return None;
    }
    let runs = crate::signaling::commands_run(command);
    for (kind, list) in [
        (DataReadKind::Production, &policy.production_reads),
        (DataReadKind::Staging, &policy.staging_reads),
    ] {
        for entry in list {
            if runs.iter().any(|run| entry_matches(entry, run)) {
                return Some(DataRead { kind, entry: entry.clone() });
            }
        }
    }
    None
}

fn entry_matches(entry: &str, run: &crate::signaling::RunCommand) -> bool {
    let words: Vec<&str> = entry.split_whitespace().collect();
    let Some((tool, rest)) = words.split_first() else {
        return false;
    };
    let tool = tool.rsplit('/').next().unwrap_or(tool);
    if run.tool != tool {
        return false;
    }
    let rest: Vec<&str> = rest.iter().copied().filter(|w| !w.starts_with('-')).collect();
    let Some((last, middle)) = rest.split_last() else {
        return true; // the tool alone
    };
    let mut args = run.args.iter();
    for word in middle {
        if !args.any(|a| a == word) {
            return false;
        }
    }
    run.args.iter().chain(run.prefix.iter()).any(|w| w.contains(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            production_reads: vec![
                "gcloud logging read".into(),
                "gcloud run".into(),
                "bq".into(),
                "psql -h ep-solitary-field-".into(),
            ],
            staging_reads: vec!["psql -h ep-young-glitter-".into()],
            ..Policy::default()
        }
    }

    fn hit(command: &str) -> Option<(DataReadKind, String)> {
        match_command(&policy(), command).map(|d| (d.kind, d.entry))
    }

    #[test]
    fn a_listed_read_matches_where_the_command_runs() {
        let prod = |e: &str| Some((DataReadKind::Production, e.to_string()));
        for (command, want) in [
            ("gcloud logging read 'resource.type=cloud_run_job' --limit 5", prod("gcloud logging read")),
            ("gcloud --project p logging read x", prod("gcloud logging read")),
            ("gcloud run jobs describe export", prod("gcloud run")),
            ("bq query --use_legacy_sql=false 'SELECT 1'", prod("bq")),
            ("psql -U u -h ep-solitary-field-a5wk.aws.pg.laravel.cloud -d db", prod("psql -h ep-solitary-field-")),
            ("psql --host=ep-solitary-field-a5wk.aws -d db", prod("psql -h ep-solitary-field-")),
            ("psql -hep-solitary-field-a5wk.aws", prod("psql -h ep-solitary-field-")),
            ("psql postgres://u@ep-solitary-field-a5wk.aws/db -c 'select 1'", prod("psql -h ep-solitary-field-")),
            ("PGHOST=ep-solitary-field-a5wk.aws psql -c 'select 1'", prod("psql -h ep-solitary-field-")),
            ("bash -c 'gcloud logging read x | head'", prod("gcloud logging read")),
            ("echo $(bq ls)", prod("bq")),
            ("/opt/homebrew/bin/gcloud run jobs list", prod("gcloud run")),
        ] {
            assert_eq!(hit(command), want, "{command}");
        }
        assert_eq!(
            hit("psql -h ep-young-glitter-a5po.aws -c 'select 1'"),
            Some((DataReadKind::Staging, "psql -h ep-young-glitter-".into()))
        );
    }

    /// Text that names a listed command without running it does not match —
    /// the user's "matched against the command a line actually runs".
    #[test]
    fn text_that_only_mentions_a_listed_read_does_not_match() {
        for command in [
            "grep -rn 'gcloud logging read' notes.md",
            "echo \"run psql -h ep-solitary-field-x later\"",
            "gcloud logging write my-log hello",
            "gcloud auth login",
            "psql -h localhost -d dev",
            "cat runbook.md",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
        assert_eq!(match_command(&Policy::default(), "bq ls"), None, "no lists, no match");
    }

    /// Agents are told the lists, and a project's list replaces the general
    /// one (the same tier rule as `per_action_approval`).
    #[test]
    fn the_lists_are_rendered_into_the_prompt_and_merged_by_tier() {
        let block = policy().render_system_prompt_block();
        assert!(block.contains("### Data reads (the user's approval, every time)"), "{block}");
        assert!(block.contains("- production: `gcloud logging read`") && block.contains("- staging: `psql -h ep-young-glitter-`"), "{block}");
        assert!(block.contains("`read_gate`"), "{block}");
        let general = Policy { production_reads: vec!["bq".into()], ..Policy::default() };
        let project = Policy { production_reads: vec!["gcloud run".into()], ..Policy::default() };
        assert_eq!(super::super::merge(general.clone(), Some(project)).production_reads, vec!["gcloud run"]);
        assert_eq!(super::super::merge(general, Some(Policy::default())).production_reads, vec!["bq"]);
    }
}
