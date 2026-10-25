//! Phase 18 integration tests (Parity Lab): semantic DataModel diff,
//! engine image diff with heatmaps, template-vs-fixture comparison and
//! the offline compatibility dashboard.

use std::path::PathBuf;
use std::process::Command;

fn engine_path() -> PathBuf {
    if let Ok(p) = std::env::var("BLOX_ENGINE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/engine/blox-engine");
    assert!(candidate.is_file(), "blox-engine not found at {}", candidate.display());
    candidate
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli")
}

fn cli_ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(out.status.success(), "bloxcli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn cli_json(args: &[&str]) -> serde_json::Value {
    let (ok, value) = cli_json_raw(args);
    assert!(ok, "bloxcli --json {args:?} failed: {value}");
    value
}

fn cli_json_raw(args: &[&str]) -> (bool, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_bloxcli"))
        .arg("--json")
        .arg("--engine")
        .arg(engine_path())
        .args(args)
        .output()
        .expect("run bloxcli --json");
    let value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.success(), value)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bloxcli-phase18-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn sample_place(dir: &PathBuf, name: &str) -> String {
    let file = dir.join(name);
    let f = file.to_str().unwrap().to_string();
    cli_ok(&["new", &f]);
    cli_ok(&["add", &f, "--parent", "Workspace", "--class", "Part", "--name", "Crate"]);
    cli_ok(&["set", &f, "--target", "Workspace/Crate", "--property", "Size", "--type", "Vector3", "--value", "4,4,4"]);
    f
}

#[test]
fn diff_reports_property_and_structure_changes() {
    let dir = temp_dir("diff");
    let a = sample_place(&dir, "a.rbxl");
    let b_path = dir.join("b.rbxl");
    std::fs::copy(&a, &b_path).unwrap();
    let b = b_path.to_str().unwrap().to_string();

    let (ok, same) = cli_json_raw(&["diff", &a, &b]);
    assert!(ok, "identical files diff clean: {same}");
    assert_eq!(same["differenceCount"], 0);

    cli_ok(&["set", &b, "--target", "Workspace/Crate", "--property", "Size", "--type", "Vector3", "--value", "9,4,4"]);
    cli_ok(&["add", &b, "--parent", "Workspace", "--class", "Part", "--name", "Extra"]);
    let (ok, diff) = cli_json_raw(&["diff", &a, &b]);
    assert!(!ok, "changed files must exit nonzero");
    let kinds: Vec<&str> = diff["differences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"property_changed"), "size change reported: {diff}");
    assert!(kinds.contains(&"added"), "new part reported: {diff}");
    let size_change = diff["differences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "property_changed")
        .unwrap()
        .clone();
    assert_eq!(size_change["property"], "Size");
    assert!(format!("{}", size_change["before"]).contains('4'), "{size_change}");
    assert!(format!("{}", size_change["after"]).contains('9'), "{size_change}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn template_compare_matches_generated_fixture() {
    let dir = temp_dir("templatecompare");
    let fixture = dir.join("fixture.rbxl");
    let ff = fixture.to_str().unwrap().to_string();
    cli_ok(&["new", &ff, "--template", "baseplate"]);

    let (ok, report) = cli_json_raw(&["template", "compare", &ff, "--template", "baseplate"]);
    assert!(ok, "a generated baseplate matches its own template: {report}");
    assert_eq!(report["differenceCount"], 0);
    assert!(report["status"].as_str().unwrap().contains("UNTESTED"), "provenance stays honest");

    // A modified fixture diverges and is reported, not hidden.
    cli_ok(&["delete", &ff, "--target", "Workspace/SpawnLocation"]);
    let (ok, report) = cli_json_raw(&["template", "compare", &ff, "--template", "baseplate"]);
    assert!(!ok);
    assert!(report["differenceCount"].as_u64().unwrap() >= 1, "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imagediff_metrics_and_heatmap() {
    let dir = temp_dir("imagediff");
    let f = sample_place(&dir, "scene.rbxl");
    cli_ok(&["move", &f, "--target", "Workspace/Crate", "--position", "0,2,0"]);
    let a_png = dir.join("a.png");
    let b_png = dir.join("b.png");
    let c_png = dir.join("c.png");
    let render = |out: &PathBuf, cam: &str| {
        cli_json(&[
            "render", &f, "--output", out.to_str().unwrap(),
            "--width", "320", "--height", "240", "--camera", cam, "--look-at", "0,2,0",
        ])
    };
    render(&a_png, "20,14,20");
    render(&b_png, "20,14,20");
    render(&c_png, "-20,14,-20");

    let same = cli_json(&["imagediff", a_png.to_str().unwrap(), b_png.to_str().unwrap()]);
    assert_eq!(same["metrics"]["mae"], 0.0, "deterministic renders are identical: {same}");
    assert_eq!(same["metrics"]["diffFraction"], 0.0);

    let heat = dir.join("heat.png");
    let (ok, diff) = cli_json_raw(&[
        "imagediff", a_png.to_str().unwrap(), c_png.to_str().unwrap(),
        "--heatmap", heat.to_str().unwrap(),
    ]);
    assert!(!ok, "different cameras must fail at zero tolerance");
    assert!(diff["metrics"]["mae"].as_f64().unwrap() > 0.0, "{diff}");
    assert!(diff["metrics"]["diffFraction"].as_f64().unwrap() > 0.001, "{diff}");
    let bytes = std::fs::read(&heat).expect("heatmap written");
    assert!(&bytes[0..8] == b"\x89PNG\r\n\x1a\n", "heatmap is a genuine PNG");

    // A generous tolerance passes the same pair.
    let tolerant = cli_json(&[
        "imagediff", a_png.to_str().unwrap(), c_png.to_str().unwrap(), "--tolerance", "1",
    ]);
    assert_eq!(tolerant["ok"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parity_dashboard_renders_report() {
    let dir = temp_dir("dashboard");
    let report_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compat-report.json");
    let report_copy = dir.join("compat-report.json");
    std::fs::copy(&report_src, &report_copy).unwrap();
    let out = dir.join("dashboard.html");
    let result = cli_json(&[
        "parity", "dashboard",
        "--report", report_copy.to_str().unwrap(),
        "--output", out.to_str().unwrap(),
    ]);
    assert_eq!(result["ok"], true);
    let html = std::fs::read_to_string(&out).expect("dashboard written");
    assert!(html.contains("compatibility dashboard"), "html header");
    assert!(html.contains("INTERNAL_VERIFIED") || html.contains("VERIFIED"), "statuses rendered: {}", &html[..400.min(html.len())]);
    assert!(html.contains("No Roblox Studio reference data"), "honest disclaimer present");
    let _ = std::fs::remove_dir_all(&dir);
}
