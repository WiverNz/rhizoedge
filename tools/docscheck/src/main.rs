//! Planning-artefact consistency validator for Rhizo Edge.
//!
//! Verifies that the planning documentation is internally consistent: required
//! artefacts exist, identifiers are unique, every cross-reference resolves, the
//! issue dependency graph is acyclic, and issue numbering is a valid execution
//! order.
//!
//! Deliberately dependency-free so it runs offline with nothing but rustc.
//!
//! Usage (from the repository root):
//!     cargo run --manifest-path tools/docscheck/Cargo.toml
//!
//! Exit code 0 = clean, 1 = at least one failure. Every failure is reported in
//! a single run: a tool that surfaces one problem per invocation stops getting
//! run.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MILESTONES: usize = 15; // M0..M14
const SAFETY_INVARIANTS: usize = 20; // SAFETY-001..020
const REQUIRED_ADRS: usize = 17; // ADR-001..017

/// Files that are historical inputs rather than maintained artefacts. Their
/// illustrative examples (e.g. "M1-003 -> M1-004 -> M2-002") are not real
/// references and must not be validated as such.
const SOURCE_INPUTS: &[&str] = &[
    "Rhizo_Edge_Claude_Code_Planning_Prompt.md",
    "Rhizo_Edge_Claude_Code_Implementation_Prompt.md",
];

const REQUIRED_ARCHITECTURE: &[&str] = &[
    "system-overview.md",
    "component-model.md",
    "data-flow.md",
    "deployment-model.md",
    "safety-invariants.md",
    "failure-model.md",
    "dependency-graph.md",
    "time-model.md",
    "configuration-model.md",
    "connectivity-modes.md",
    "offline-autonomy.md",
];

const REQUIRED_PROTOCOL: &[&str] = &[
    "mqtt-v1.md",
    "http-api-boundaries.md",
    "versioning-policy.md",
];

const REQUIRED_TESTING: &[&str] = &[
    "strategy.md",
    "simulator-strategy.md",
    "failure-scenarios.md",
    "hardware-in-the-loop.md",
    "local-development.md",
];

struct Report {
    failures: Vec<String>,
    checks: usize,
}

impl Report {
    fn new() -> Self {
        Report {
            failures: Vec::new(),
            checks: 0,
        }
    }
    fn check(&mut self, ok: bool, msg: impl Into<String>) {
        self.checks += 1;
        if !ok {
            self.failures.push(msg.into());
        }
    }
    fn fail(&mut self, msg: impl Into<String>) {
        self.checks += 1;
        self.failures.push(msg.into());
    }
}

fn main() {
    let root = repo_root();
    let mut r = Report::new();

    let docs = collect_markdown(&root);

    let issues = check_issues(&root, &mut r);
    let adrs = check_adrs(&root, &mut r);
    let prds = check_prds(&root, &mut r);
    let safety = check_safety_registry(&root, &mut r);
    let scen = collect_ids(&root.join("docs/testing/failure-scenarios.md"), "SCEN-");

    check_required_files(&root, &mut r);
    check_roadmap(&root, &issues, &mut r);
    check_references(&root, &docs, &issues, &adrs, &prds, &safety, &scen, &mut r);
    check_links(&root, &docs, &mut r);
    check_dependency_graph(&root, &issues, &mut r);

    println!("rhizo-docscheck — planning artefact validation");
    println!(
        "  artefacts: {} issues, {} ADRs, {} PRDs, {} invariants, {} scenarios, {} markdown files",
        issues.values().map(|v| v.len()).sum::<usize>(),
        adrs.len(),
        prds.len(),
        safety.len(),
        scen.len(),
        docs.len()
    );
    println!("  checks run: {}", r.checks);

    if r.failures.is_empty() {
        println!("\nOK — no inconsistencies found.");
    } else {
        println!("\n{} FAILURE(S):\n", r.failures.len());
        for f in &r.failures {
            println!("  - {f}");
        }
        std::process::exit(1);
    }
}

fn repo_root() -> PathBuf {
    // Run from the repository root, or from a subdirectory such as
    // tools/docscheck when invoked via --manifest-path.
    //
    // A working directory that cannot be read is not worth panicking over: the
    // repository root is more usefully looked for from `.` than not at all.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("docs").is_dir() && cwd.join("ROADMAP.md").is_file() {
        return cwd;
    }
    let mut p = cwd.as_path();
    while let Some(parent) = p.parent() {
        if parent.join("docs").is_dir() && parent.join("ROADMAP.md").is_file() {
            return parent.to_path_buf();
        }
        p = parent;
    }
    cwd
}

// ---------------------------------------------------------------- collection

fn collect_markdown(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for top in ["ROADMAP.md", "README.md", "CLAUDE.md"] {
        let p = root.join(top);
        if p.is_file() {
            out.push(p);
        }
    }
    walk(&root.join("docs"), &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().map(|x| x == "md").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn is_source_input(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| SOURCE_INPUTS.contains(&n))
        .unwrap_or(false)
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .display()
        .to_string()
        .replace('\\', "/")
}

/// Extract three-digit ids following a prefix, e.g. "SAFETY-" -> {"001", ...}.
fn collect_ids(path: &Path, prefix: &str) -> BTreeSet<String> {
    let txt = read(path);
    let mut out = BTreeSet::new();
    let bytes: Vec<char> = txt.chars().collect();
    let pat: Vec<char> = prefix.chars().collect();
    let mut i = 0;
    while i + pat.len() + 3 <= bytes.len() {
        if bytes[i..i + pat.len()] == pat[..] {
            let d = &bytes[i + pat.len()..i + pat.len() + 3];
            if d.iter().all(|c| c.is_ascii_digit()) {
                out.insert(d.iter().collect::<String>());
            }
        }
        i += 1;
    }
    out
}

// -------------------------------------------------------------------- checks

/// Returns milestone -> set of three-digit issue numbers.
fn check_issues(root: &Path, r: &mut Report) -> BTreeMap<usize, BTreeSet<String>> {
    let mut map = BTreeMap::new();
    for m in 0..MILESTONES {
        let dir = root.join(format!("docs/issues/M{m}"));
        if !dir.is_dir() {
            r.fail(format!("missing issue directory docs/issues/M{m}"));
            map.insert(m, BTreeSet::new());
            continue;
        }
        let mut nums = BTreeSet::new();
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".md") {
                continue;
            }
            if name.len() < 4 || !name[..3].chars().all(|c| c.is_ascii_digit()) {
                r.fail(format!(
                    "M{m}: issue file '{name}' does not start with a 3-digit id"
                ));
                continue;
            }
            let num = name[..3].to_string();
            if let Some(prev) = seen.insert(num.clone(), name.clone()) {
                r.fail(format!(
                    "M{m}: duplicate issue id {num} ({prev} and {name})"
                ));
            }
            nums.insert(num);
        }
        r.check(!nums.is_empty(), format!("M{m}: milestone has no issues"));
        map.insert(m, nums);
    }
    map
}

fn check_adrs(root: &Path, r: &mut Report) -> BTreeSet<String> {
    let dir = root.join("docs/adr");
    let mut ids = BTreeSet::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        r.fail("missing docs/adr directory");
        return ids;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        if name.len() < 4 || !name[..3].chars().all(|c| c.is_ascii_digit()) {
            r.fail(format!(
                "ADR file '{name}' does not start with a 3-digit id"
            ));
            continue;
        }
        let id = name[..3].to_string();
        if let Some(prev) = seen.insert(id.clone(), name.clone()) {
            r.fail(format!("duplicate ADR id {id} ({prev} and {name})"));
        }
        ids.insert(id);
    }
    // ADR-001..017: the planning-prompt minimum plus those added since.
    for n in 1..=REQUIRED_ADRS {
        let id = format!("{n:03}");
        r.check(ids.contains(&id), format!("required ADR-{id} is missing"));
    }
    ids
}

fn check_prds(root: &Path, r: &mut Report) -> BTreeSet<String> {
    let dir = root.join("docs/prd");
    let mut ids = BTreeSet::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        r.fail("missing docs/prd directory");
        return ids;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        if name.len() < 4 || !name[..3].chars().all(|c| c.is_ascii_digit()) {
            r.fail(format!(
                "PRD file '{name}' does not start with a 3-digit id"
            ));
            continue;
        }
        let id = name[..3].to_string();
        if let Some(prev) = seen.insert(id.clone(), name.clone()) {
            r.fail(format!("duplicate PRD id {id} ({prev} and {name})"));
        }
        ids.insert(id);
    }
    // One PRD per milestone: 000, 010, ... 140.
    for m in 0..MILESTONES {
        let id = format!("{:03}", m * 10);
        r.check(
            ids.contains(&id),
            format!("required PRD {id} (milestone M{m}) is missing"),
        );
    }
    ids
}

fn check_safety_registry(root: &Path, r: &mut Report) -> BTreeSet<String> {
    let path = root.join("docs/architecture/safety-invariants.md");
    if !path.is_file() {
        r.fail("missing docs/architecture/safety-invariants.md");
        return BTreeSet::new();
    }
    let txt = read(&path);
    let ids = collect_ids(&path, "SAFETY-");
    for n in 1..=SAFETY_INVARIANTS {
        let id = format!("{n:03}");
        r.check(
            ids.contains(&id),
            format!("SAFETY-{id} is missing from the registry"),
        );
        // Each invariant needs its own section and a planned test.
        let heading = format!("## SAFETY-{id}");
        if txt.contains(&heading) {
            let after = txt.split(&heading).nth(1).unwrap_or("");
            let section = after.split("\n## ").next().unwrap_or("");
            r.check(
                section.contains("Planned tests"),
                format!("SAFETY-{id} has no 'Planned tests' section"),
            );
        } else {
            r.fail(format!("SAFETY-{id} has no '## SAFETY-{id}' section"));
        }
    }
    ids
}

fn check_required_files(root: &Path, r: &mut Report) {
    for f in REQUIRED_ARCHITECTURE {
        let p = root.join("docs/architecture").join(f);
        r.check(p.is_file(), format!("missing docs/architecture/{f}"));
    }
    for f in REQUIRED_PROTOCOL {
        let p = root.join("docs/protocol").join(f);
        r.check(p.is_file(), format!("missing docs/protocol/{f}"));
    }
    for f in REQUIRED_TESTING {
        let p = root.join("docs/testing").join(f);
        r.check(p.is_file(), format!("missing docs/testing/{f}"));
    }
    for f in ["ROADMAP.md", "README.md", "docs/README.md"] {
        r.check(root.join(f).is_file(), format!("missing {f}"));
    }
}

fn check_roadmap(root: &Path, issues: &BTreeMap<usize, BTreeSet<String>>, r: &mut Report) {
    let path = root.join("ROADMAP.md");
    if !path.is_file() {
        return; // already reported
    }
    let txt = read(&path);
    for m in 0..MILESTONES {
        // Match "| M0 |" or "### M0 —" so a stray "M0-001" does not satisfy it.
        let table = format!("| M{m} |");
        let heading = format!("### M{m} ");
        r.check(
            txt.contains(&table) || txt.contains(&heading),
            format!("ROADMAP.md does not list milestone M{m}"),
        );
    }
    // Issue counts stated in the roadmap table must match reality.
    for (m, nums) in issues {
        let claim = format!("| {} | ", nums.len());
        let row_marker = format!("| M{m} |");
        if let Some(line) = txt.lines().find(|l| l.starts_with(&row_marker)) {
            r.check(
                line.contains(&claim),
                format!(
                    "ROADMAP.md M{m} row does not state the real issue count ({})",
                    nums.len()
                ),
            );
        }
    }
    let total: usize = issues.values().map(|v| v.len()).sum();
    r.check(
        txt.contains(&format!("{total} issues")),
        format!("ROADMAP.md does not state the real total issue count ({total})"),
    );
}

#[allow(clippy::too_many_arguments)]
fn check_references(
    root: &Path,
    docs: &[PathBuf],
    issues: &BTreeMap<usize, BTreeSet<String>>,
    adrs: &BTreeSet<String>,
    prds: &BTreeSet<String>,
    safety: &BTreeSet<String>,
    scen: &BTreeSet<String>,
    r: &mut Report,
) {
    for doc in docs {
        if is_source_input(doc) {
            continue;
        }
        let txt = read(doc);
        let name = rel(root, doc);
        let chars: Vec<char> = txt.chars().collect();

        for (i, _) in chars.iter().enumerate() {
            // M<digits>-<3 digits>
            if chars[i] == 'M' && i + 2 < chars.len() && chars[i + 1].is_ascii_digit() {
                // avoid matching inside a word
                if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '-') {
                    continue;
                }
                let mut j = i + 1;
                let mut num = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() && num.len() < 2 {
                    num.push(chars[j]);
                    j += 1;
                }
                if j < chars.len() && chars[j] == '-' {
                    let d: String = chars[j + 1..].iter().take(3).collect();
                    if d.len() == 3 && d.chars().all(|c| c.is_ascii_digit()) {
                        let after = chars.get(j + 4).copied().unwrap_or(' ');
                        if !after.is_ascii_digit()
                            && let Ok(mi) = num.parse::<usize>()
                        {
                            let exists = issues.get(&mi).is_some_and(|s| s.contains(&d));
                            r.check(
                                exists,
                                format!("{name}: references M{mi}-{d}, which does not exist"),
                            );
                        }
                    }
                }
            }
        }

        check_prefixed(&txt, "ADR-", adrs, &name, "ADR", r);
        check_prefixed(&txt, "SAFETY-", safety, &name, "safety invariant", r);
        check_prefixed(&txt, "SCEN-", scen, &name, "scenario", r);
        check_prefixed(&txt, "PRD ", prds, &name, "PRD", r);
    }
}

fn check_prefixed(
    txt: &str,
    prefix: &str,
    known: &BTreeSet<String>,
    doc: &str,
    kind: &str,
    r: &mut Report,
) {
    let chars: Vec<char> = txt.chars().collect();
    let pat: Vec<char> = prefix.chars().collect();
    let mut seen = BTreeSet::new();
    let mut i = 0;
    while i + pat.len() + 3 <= chars.len() {
        if chars[i..i + pat.len()] == pat[..] {
            let d: String = chars[i + pat.len()..i + pat.len() + 3].iter().collect();
            if d.chars().all(|c| c.is_ascii_digit()) {
                let after = chars.get(i + pat.len() + 3).copied().unwrap_or(' ');
                if !after.is_ascii_digit() {
                    seen.insert(d);
                }
            }
        }
        i += 1;
    }
    for d in seen {
        r.check(
            known.contains(&d),
            format!("{doc}: references {kind} {prefix}{d}, which does not exist"),
        );
    }
}

fn check_links(root: &Path, docs: &[PathBuf], r: &mut Report) {
    for doc in docs {
        if is_source_input(doc) {
            continue;
        }
        // Strip fenced code blocks: a "](" inside a regex or a shell snippet is
        // not a link, and flagging it would train people to ignore this tool.
        let txt = strip_code_fences(&read(doc));
        let dir = doc.parent().unwrap_or(root);
        let name = rel(root, doc);
        let bytes: Vec<char> = txt.chars().collect();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == ']' && bytes[i + 1] == '(' {
                let mut j = i + 2;
                let mut target = String::new();
                let mut depth = 0;
                while j < bytes.len() {
                    match bytes[j] {
                        '(' => {
                            depth += 1;
                            target.push('(');
                        }
                        ')' if depth == 0 => break,
                        ')' => {
                            depth -= 1;
                            target.push(')');
                        }
                        c => target.push(c),
                    }
                    j += 1;
                }
                let t = target.trim();
                let skip = t.is_empty()
                    || t.starts_with("http://")
                    || t.starts_with("https://")
                    || t.starts_with("mailto:")
                    || t.starts_with('#')
                    || t.starts_with('<');
                if !skip {
                    let path_part = t.split('#').next().unwrap_or(t);
                    if !path_part.is_empty() {
                        let resolved = dir.join(path_part);
                        r.check(
                            resolved.exists(),
                            format!("{name}: broken link -> {path_part}"),
                        );
                    }
                }
                i = j;
            }
            i += 1;
        }
    }
}

/// Remove ``` fenced blocks and `inline code` spans, preserving line structure.
fn strip_code_fences(txt: &str) -> String {
    let mut out = String::with_capacity(txt.len());
    let mut in_fence = false;
    for line in txt.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push('\n');
            continue;
        }
        // Drop inline code spans on this line.
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
                out.push(' ');
            } else {
                out.push(if in_code { ' ' } else { c });
            }
        }
        out.push('\n');
    }
    out
}

/// Parse each issue's `**Depends on:**` header and verify the graph is sane.
fn check_dependency_graph(root: &Path, issues: &BTreeMap<usize, BTreeSet<String>>, r: &mut Report) {
    let mut deps: BTreeMap<(usize, String), Vec<(usize, String)>> = BTreeMap::new();

    for (m, nums) in issues {
        for num in nums {
            let dir = root.join(format!("docs/issues/M{m}"));
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let Some(file) = entries.flatten().map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(num.as_str()))
                    .unwrap_or(false)
            }) else {
                continue;
            };
            let txt = read(&file);
            let Some(line) = txt.lines().find(|l| l.contains("**Depends on:**")) else {
                r.fail(format!("M{m}-{num}: no '**Depends on:**' header"));
                continue;
            };
            let after = line.split("**Depends on:**").nth(1).unwrap_or("").trim();
            let mut list = Vec::new();
            if after != "—" && !after.is_empty() {
                for tok in after.split(',') {
                    let tok = tok.trim();
                    if let Some((ms, is)) = parse_issue_id(tok) {
                        let exists = issues.get(&ms).map(|s| s.contains(&is)).unwrap_or(false);
                        r.check(
                            exists,
                            format!("M{m}-{num}: depends on M{ms}-{is}, which does not exist"),
                        );
                        if exists {
                            // Within a milestone, dependencies must have lower
                            // numbers, so numeric order is a valid execution order.
                            if ms == *m {
                                r.check(
                                    is.as_str() < num.as_str(),
                                    format!(
                                        "M{m}-{num}: depends on M{ms}-{is}, which is not earlier in the same milestone"
                                    ),
                                );
                            } else {
                                r.check(
                                    ms < *m,
                                    format!(
                                        "M{m}-{num}: depends on M{ms}-{is} from a LATER milestone"
                                    ),
                                );
                            }
                            list.push((ms, is));
                        }
                    }
                }
            }
            deps.insert((*m, num.clone()), list);
        }
    }

    // Acyclicity. The intra-milestone ordering rule above already forbids cycles,
    // but verify directly so the guarantee does not depend on that reasoning.
    let mut state: BTreeMap<(usize, String), u8> = BTreeMap::new();
    let nodes: Vec<_> = deps.keys().cloned().collect();
    for n in nodes {
        if let Some(cycle) = visit(&n, &deps, &mut state) {
            r.fail(format!("dependency cycle: {cycle}"));
        }
    }
}

fn visit(
    node: &(usize, String),
    deps: &BTreeMap<(usize, String), Vec<(usize, String)>>,
    state: &mut BTreeMap<(usize, String), u8>,
) -> Option<String> {
    match state.get(node) {
        Some(1) => return Some(format!("M{}-{}", node.0, node.1)),
        Some(2) => return None,
        _ => {}
    }
    state.insert(node.clone(), 1);
    if let Some(children) = deps.get(node) {
        for c in children {
            if let Some(path) = visit(c, deps, state) {
                return Some(format!("M{}-{} -> {}", node.0, node.1, path));
            }
        }
    }
    state.insert(node.clone(), 2);
    None
}

fn parse_issue_id(tok: &str) -> Option<(usize, String)> {
    let t = tok.trim().trim_matches('`').trim_matches('*');
    if !t.starts_with('M') {
        return None;
    }
    let rest = &t[1..];
    let dash = rest.find('-')?;
    let ms: usize = rest[..dash].parse().ok()?;
    let num: String = rest[dash + 1..].chars().take(3).collect();
    if num.len() == 3 && num.chars().all(|c| c.is_ascii_digit()) {
        Some((ms, num))
    } else {
        None
    }
}
