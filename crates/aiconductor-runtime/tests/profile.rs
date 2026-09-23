//! Keeps docs/PROFILE_v0.1_ja.md (normative) and its translation honest: the matrix is the single status record,
//! so every implementation claim must name tests that exist in this workspace.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const STATUS: [&str; 4] = ["yes", "no", "partial", "n/a"];

struct Row {
    feature: String,
    profile: String,
    implemented: [String; 3],
    tested: String,
    evidence_text: String,
    evidence: Vec<String>,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The table lines of a profile document, from its header row to its end.
fn table_lines(file: &str) -> Vec<String> {
    let profile = fs::read_to_string(workspace().join("docs").join(file)).unwrap();
    profile
        .lines()
        .skip_while(|line| !line.starts_with("| Feature"))
        .take_while(|line| line.starts_with('|'))
        .map(str::to_owned)
        .collect()
}

/// The Japanese profile is normative; its matrix is parsed.
fn matrix() -> Vec<Row> {
    table_lines("PROFILE_v0.1_ja.md")
        .iter()
        .skip(2)
        .map(|line| {
            let cells = line
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(cells.len(), 8, "malformed row: {line}");
            let evidence = cells[7]
                .split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect();
            Row {
                feature: cells[0].clone(),
                profile: cells[2].clone(),
                implemented: [cells[3].clone(), cells[4].clone(), cells[5].clone()],
                tested: cells[6].clone(),
                evidence_text: cells[7].clone(),
                evidence,
            }
        })
        .collect()
}

fn test_names() -> BTreeSet<String> {
    fn visit(dir: &Path, names: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
                visit(&path, names);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                let mut lines = source.lines().peekable();
                while let Some(line) = lines.next() {
                    if line.trim_start().starts_with("#[test]")
                        || line.trim_start().starts_with("#[tokio::test]")
                    {
                        let signature = lines.next().unwrap_or_default();
                        if let Some(name) = signature
                            .split("fn ")
                            .nth(1)
                            .and_then(|rest| rest.split('(').next())
                        {
                            names.insert(name.trim().to_owned());
                        }
                    }
                }
            }
        }
    }
    let mut names = BTreeSet::new();
    visit(&workspace().join("crates"), &mut names);
    names
}

#[test]
fn profile_matrix_is_backed_by_existing_tests() {
    // The English reference translation must carry the identical matrix.
    let (japanese, english) = (
        table_lines("PROFILE_v0.1_ja.md"),
        table_lines("PROFILE_v0.1.md"),
    );
    assert_eq!(
        japanese.len(),
        english.len(),
        "profile matrices differ in length"
    );
    for (ja, en) in japanese.iter().zip(&english) {
        assert_eq!(ja, en, "Japanese and English profile matrices differ");
    }
    let rows = matrix();
    let tests = test_names();
    assert!(rows.len() > 30, "matrix unexpectedly short");
    let mut features = BTreeSet::new();
    for row in &rows {
        assert!(features.insert(&row.feature), "duplicate {}", row.feature);
        assert!(
            ["v0.1", "v0.2"].contains(&row.profile.as_str()),
            "{}: profile {}",
            row.feature,
            row.profile
        );
        for value in row.implemented.iter().chain([&row.tested]) {
            assert!(STATUS.contains(&value.as_str()), "{}: {value}", row.feature);
        }
        // A full claim needs tests; an untested partial claim must explain itself.
        if row.implemented.iter().any(|value| value == "yes") {
            assert_ne!(
                row.tested, "no",
                "{} claims code without tests",
                row.feature
            );
        }
        if row.implemented.iter().any(|value| value == "partial") && row.tested == "no" {
            assert_ne!(
                row.evidence_text, "—",
                "{} needs an explanation",
                row.feature
            );
        }
        if row.tested != "no" {
            assert!(!row.evidence.is_empty(), "{} has no evidence", row.feature);
        }
        for name in &row.evidence {
            assert!(tests.contains(name), "{}: unknown test {name}", row.feature);
        }
    }
}
