use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

fn altaika() -> Command {
    Command::new(env!("CARGO_BIN_EXE_altaika"))
}

fn temp_csv(name: &str, contents: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "altaika-{name}-{}-{}.csv",
        std::process::id(),
        chrono_like_counter()
    ));
    fs::write(&path, contents).expect("write csv fixture");
    path.to_string_lossy().into_owned()
}

fn temp_path(name: &str, extension: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "altaika-{name}-{}-{}.{}",
            std::process::id(),
            chrono_like_counter(),
            extension
        ))
        .to_string_lossy()
        .into_owned()
}

fn temp_extensionless_file(name: &str, contents: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "altaika-{name}-{}-{}",
        std::process::id(),
        chrono_like_counter()
    ));
    fs::write(&path, contents).expect("write fixture");
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn temp_executable(name: &str, contents: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "altaika-{name}-{}-{}",
        std::process::id(),
        chrono_like_counter()
    ));
    fs::write(&path, contents).expect("write executable fixture");
    let mut permissions = fs::metadata(&path).expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod fixture");
    path.to_string_lossy().into_owned()
}

fn chrono_like_counter() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos()
}

#[derive(Debug, Deserialize)]
struct ConformanceCase {
    command: String,
    path: String,
    expected_kind: String,
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    filters: Vec<String>,
    limit: Option<usize>,
}

fn conformance_case(name: &str) -> ConformanceCase {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/cases")
        .join(format!("{name}.json"));
    let contents = fs::read_to_string(&path).expect("read conformance case");
    serde_json::from_str(&contents).expect("parse conformance case")
}

#[test]
fn engine_docs_include_quack_remote_transport_lane() {
    let docs_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/engines");
    let quack = fs::read_to_string(docs_root.join("quack.md")).expect("read quack docs");
    let duckdb = fs::read_to_string(docs_root.join("duckdb.md")).expect("read duckdb docs");

    assert!(quack.contains("beta support through `--engine duckdb-beta`"));
    assert!(quack.contains("DuckDB v1.5.3"));
    assert!(quack.contains("ALTAIKA_DUCKDB_BIN"));
    assert!(quack.contains("explicit SQL and auth diagnostics only"));
    assert!(quack.contains("QUACK_SERVER_URI"));
    assert!(quack.contains("not a SQL dialect"));
    assert!(duckdb.contains("Quack"));
}

#[test]
fn sql_select_one_returns_json_rows() {
    let output = altaika()
        .args(["sql", "SELECT 1 AS one"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "rows");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["schema_version"], "1.0");
    assert_eq!(body["data"], serde_json::json!([{ "one": 1 }]));
    assert_eq!(body["meta"]["row_count"], 1);
    assert_eq!(body["meta"]["limit"], 500);
}

#[test]
fn sql_defaults_to_bounded_rows() {
    let mut contents = "id\n".to_string();
    for id in 0..501 {
        contents.push_str(&format!("{id}\n"));
    }
    let csv_path = temp_csv("many-events", &contents);
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "sql",
            "SELECT id FROM public.events",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["data"].as_array().expect("rows").len(), 500);
}

#[test]
fn sql_limit_can_override_default_bound() {
    let csv_path = temp_csv("events", "id\n1\n2\n3\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "sql",
            "--limit",
            "2",
            "SELECT id FROM public.events",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["data"], serde_json::json!([{ "id": 1 }, { "id": 2 }]));
}

#[cfg(unix)]
#[test]
fn duckdb_beta_sql_uses_configured_duckdb_binary() {
    let duckdb = temp_executable(
        "duckdb",
        r#"#!/bin/sh
if [ "$1" = ":memory:" ] && [ "$2" = "-json" ] && [ "$3" = "-c" ]; then
  echo '[{"one":1}]'
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    );

    let output = altaika()
        .args(["--engine", "duckdb-beta", "sql", "SELECT 1 AS one"])
        .env("ALTAIKA_DUCKDB_BIN", &duckdb)
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "rows");
    assert_eq!(body["engine"], "duckdb-beta");
    assert_eq!(body["data"], serde_json::json!([{ "one": 1 }]));
}

#[cfg(unix)]
#[test]
fn duckdb_beta_auth_reports_local_quack_readiness() {
    let duckdb = temp_executable(
        "duckdb-auth",
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 'v1.5.4 (Andium) abc123'
  exit 0
fi
if [ "$1" = ":memory:" ] && [ "$2" = "-json" ] && [ "$3" = "-c" ]; then
  echo '[{"extension_name":"quack","installed":true,"loaded":false}]'
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    );

    let output = altaika()
        .args(["--engine", "duckdb-beta", "auth"])
        .env("ALTAIKA_DUCKDB_BIN", &duckdb)
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "duckdb-beta");
    assert_eq!(body["data"]["ready"], true);
    assert_eq!(body["data"]["checked"], false);
    assert_eq!(body["data"]["runtime_only"], true);
    assert_eq!(body["data"]["missing"], serde_json::json!([]));
    assert_eq!(body["data"]["present"]["ALTAIKA_DUCKDB_BIN"], true);
    assert_eq!(body["data"]["present"]["quack_extension"], true);
    assert_eq!(body["data"]["present"]["duckdb_binary"], duckdb);
    assert_eq!(
        body["data"]["present"]["duckdb_version"],
        "v1.5.4 (Andium) abc123"
    );
}

#[cfg(unix)]
#[test]
fn duckdb_beta_auth_check_marks_probe_checked() {
    let duckdb = temp_executable(
        "duckdb-auth-check",
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 'v1.5.4 (Andium) abc123'
  exit 0
fi
if [ "$1" = ":memory:" ] && [ "$2" = "-json" ] && [ "$3" = "-c" ]; then
  echo '[{"extension_name":"quack","installed":true,"loaded":false}]'
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    );

    let output = altaika()
        .args(["--engine", "duckdb-beta", "auth", "--check"])
        .env("ALTAIKA_DUCKDB_BIN", &duckdb)
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["data"]["ready"], true);
    assert_eq!(body["data"]["checked"], true);
    assert_eq!(body["data"]["connection"], "ok");
}

#[cfg(unix)]
#[test]
fn duckdb_beta_auth_requires_installed_or_loaded_quack() {
    let duckdb = temp_executable(
        "duckdb-auth-missing-quack",
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 'v1.5.4 (Andium) abc123'
  exit 0
fi
if [ "$1" = ":memory:" ] && [ "$2" = "-json" ] && [ "$3" = "-c" ]; then
  echo '[{"extension_name":"quack","installed":"false","loaded":"false"}]'
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    );

    let output = altaika()
        .args(["--engine", "duckdb-beta", "auth"])
        .env("ALTAIKA_DUCKDB_BIN", &duckdb)
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["data"]["ready"], false);
    assert_eq!(
        body["data"]["missing"],
        serde_json::json!(["quack_extension"])
    );
    assert_eq!(body["data"]["present"]["quack_extension"], false);
}

#[test]
fn datafusion_runs_conformance_cases() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("analytics.events={csv_path}");

    for name in ["ls", "describe", "cat"] {
        let case = conformance_case(name);
        let mut args = vec![
            "--csv".to_string(),
            registration.clone(),
            case.command.clone(),
            case.path.clone(),
        ];
        if !case.columns.is_empty() {
            args.extend(["--columns".to_string(), case.columns.join(",")]);
        }
        for filter in &case.filters {
            args.extend(["--filter".to_string(), filter.clone()]);
        }
        if let Some(limit) = case.limit {
            args.extend(["--limit".to_string(), limit.to_string()]);
        }

        let output = altaika().args(args).output().expect("run altaika");

        assert!(output.status.success(), "{output:?}");

        let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
        assert_eq!(body["kind"], case.expected_kind, "{name}");
        assert_eq!(body["engine"], "datafusion", "{name}");
        match case.command.as_str() {
            "ls" => assert_eq!(
                body["data"],
                serde_json::json!([{ "table_name": "events" }]),
                "{name}"
            ),
            "describe" => assert_eq!(
                body["data"],
                serde_json::json!([
                    {"name": "id", "data_type": "Int64", "nullable": true},
                    {"name": "event_name", "data_type": "Utf8", "nullable": true}
                ]),
                "{name}"
            ),
            "cat" => assert_eq!(body["data"], serde_json::json!([{ "id": 1 }]), "{name}"),
            other => panic!("unsupported conformance command {other}"),
        }
    }
}

#[test]
fn cat_reads_registered_csv() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "cat",
            "local/public/events",
            "--columns",
            "id",
            "--filter",
            "event_name:=signup",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "rows");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"], serde_json::json!([{ "id": 1 }]));
    assert_eq!(body["meta"]["row_count"], 1);
    assert_eq!(body["meta"]["limit"], 10);
    assert_eq!(
        body["meta"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true}
        ])
    );
    assert_eq!(body["meta"]["profile"], serde_json::Value::Null);
}

#[test]
fn cat_uses_public_schema_for_local_default_path() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "cat",
            "local/events",
            "--columns",
            "id",
            "--filter",
            "event_name:=signup",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["data"], serde_json::json!([{ "id": 1 }]));
}

#[test]
fn cat_filters_numeric_comparisons() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "cat",
            "local/events",
            "--columns",
            "id,event_name",
            "--filter",
            "id:>1",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(
        body["data"],
        serde_json::json!([{ "id": 2, "event_name": "login" }])
    );
}

#[test]
fn cat_can_emit_ndjson_rows() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--format",
            "ndjson",
            "--csv",
            &registration,
            "cat",
            "local/events",
            "--columns",
            "id",
            "--limit",
            "2",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        "{\"id\":1}\n{\"id\":2}\n"
    );
}

#[test]
fn snapshot_writes_parquet_and_manifest() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");
    let out = temp_path("events-snapshot", "parquet");
    let manifest_path = format!("{out}.manifest.json");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "snapshot",
            "local/events",
            "--out",
            &out,
            "--columns",
            "id",
            "--limit",
            "1",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");
    assert!(Path::new(&out).exists(), "snapshot parquet should exist");
    assert!(
        Path::new(&manifest_path).exists(),
        "snapshot manifest should exist"
    );

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "snapshot");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"]["path"], out);
    assert_eq!(body["data"]["manifest_path"], manifest_path);
    assert_eq!(body["data"]["row_count"], 1);

    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
        .expect("manifest json");
    assert_eq!(manifest["kind"], "snapshot_manifest");
    assert_eq!(manifest["source"], "local/events");
    assert_eq!(manifest["data_path"], out);
    assert_eq!(manifest["row_count"], 1);
    assert_eq!(manifest["query"], "SELECT id FROM public.events LIMIT 1");
    assert_eq!(manifest["source_profile"]["path"], "local/events");
    assert_eq!(manifest["source_profile"]["source_uri"], csv_path);
    assert_eq!(manifest["source_profile"]["source_format"], "csv");
    assert_eq!(
        manifest["source_profile"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );

    let parquet_registration = format!("public.events_snapshot={out}");
    let read_back = altaika()
        .args([
            "--parquet",
            &parquet_registration,
            "cat",
            "local/events_snapshot",
            "--columns",
            "id",
            "--limit",
            "10",
        ])
        .output()
        .expect("read snapshot");

    assert!(read_back.status.success(), "{read_back:?}");
    let body: Value = serde_json::from_slice(&read_back.stdout).expect("json stdout");
    assert_eq!(body["data"], serde_json::json!([{ "id": 1 }]));

    let described_snapshot = altaika()
        .args([
            "--parquet",
            &parquet_registration,
            "describe",
            "local/events_snapshot",
        ])
        .output()
        .expect("describe snapshot");

    assert!(
        described_snapshot.status.success(),
        "{described_snapshot:?}"
    );
    let body: Value = serde_json::from_slice(&described_snapshot.stdout).expect("json stdout");
    assert_eq!(body["profile"]["estimated_rows"], 1);
}

#[test]
fn auth_reports_datafusion_ready() {
    let output = altaika().arg("auth").output().expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"]["ready"], true);
    assert_eq!(body["data"]["checked"], false);
    assert_eq!(body["data"]["missing"], serde_json::json!([]));
}

#[test]
fn auth_check_reports_datafusion_checked() {
    let output = altaika()
        .args(["auth", "--check"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"]["ready"], true);
    assert_eq!(body["data"]["checked"], true);
    assert_eq!(body["data"]["connection"], "ok");
}

#[test]
fn auth_reports_altertable_missing_runtime_user_without_secret_values() {
    let output = altaika()
        .args(["--engine", "altertable", "auth"])
        .env_remove("ALTERTABLE_USER")
        .env("ALTERTABLE_PASSWORD", "super-secret")
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("super-secret"),
        "auth diagnostics must not print secret values"
    );

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "altertable");
    assert_eq!(body["data"]["ready"], false);
    assert_eq!(body["data"]["checked"], false);
    assert_eq!(body["data"]["runtime_only"], true);
    assert_eq!(
        body["data"]["missing"],
        serde_json::json!(["ALTERTABLE_USER"])
    );
    assert_eq!(body["data"]["present"]["ALTERTABLE_PASSWORD"], true);
}

#[test]
fn auth_check_does_not_connect_when_altertable_credentials_are_missing() {
    let output = altaika()
        .args(["--engine", "altertable", "auth", "--check"])
        .env_remove("ALTERTABLE_USER")
        .env_remove("ALTERTABLE_PASSWORD")
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "altertable");
    assert_eq!(body["data"]["ready"], false);
    assert_eq!(body["data"]["checked"], false);
    assert_eq!(body["data"]["connection"], "skipped");
    assert_eq!(
        body["data"]["missing"],
        serde_json::json!(["ALTERTABLE_USER", "ALTERTABLE_PASSWORD"])
    );
}

#[test]
fn auth_reports_altertable_missing_runtime_password() {
    let output = altaika()
        .args(["--engine", "altertable", "auth"])
        .env("ALTERTABLE_USER", "agent@example.com")
        .env_remove("ALTERTABLE_PASSWORD")
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "auth");
    assert_eq!(body["engine"], "altertable");
    assert_eq!(body["data"]["ready"], false);
    assert_eq!(body["data"]["checked"], false);
    assert_eq!(
        body["data"]["missing"],
        serde_json::json!(["ALTERTABLE_PASSWORD"])
    );
    assert_eq!(body["data"]["present"]["ALTERTABLE_USER"], true);
    assert_eq!(body["data"]["present"]["ALTERTABLE_PASSWORD"], false);
}

#[test]
fn plan_cat_renders_target_sql_without_executing() {
    let output = altaika()
        .args([
            "plan",
            "cat",
            "local/events",
            "--target",
            "snowflake",
            "--columns",
            "id,event_name",
            "--filter",
            "event_name:=signup",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"]["source_dialect"], "duckdb");
    assert_eq!(body["data"]["target_dialect"], "snowflake");
    assert_eq!(
        body["data"]["canonical_sql"],
        "SELECT id, event_name FROM public.events WHERE event_name = 'signup' LIMIT 10"
    );
    assert!(
        body["data"]["rendered_sql"]
            .as_str()
            .expect("rendered sql")
            .contains("event_name")
    );
}

#[test]
fn plan_cat_renders_ducklake_as_duckdb_compatible_sql() {
    let output = altaika()
        .args([
            "plan",
            "cat",
            "local/events",
            "--target",
            "ducklake",
            "--columns",
            "id",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["source_dialect"], "duckdb");
    assert_eq!(body["data"]["target_dialect"], "ducklake");
    assert_eq!(
        body["data"]["rendered_sql"],
        "SELECT id FROM public.events LIMIT 10"
    );
}

#[test]
fn plan_ls_renders_target_sql_without_executing() {
    let output = altaika()
        .args(["plan", "ls", "local/public", "--target", "snowflake"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["target_dialect"], "snowflake");
    assert_eq!(
        body["data"]["canonical_sql"],
        "SELECT table_name FROM information_schema.tables WHERE table_schema LIKE 'public'"
    );
}

#[test]
fn plan_ls_table_includes_registered_source_profile() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "plan",
            "ls",
            "local/public/events",
            "--target",
            "datafusion",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["profile"]["path"], "local/public/events");
    assert_eq!(body["data"]["profile"]["source_format"], "csv");
    assert_eq!(
        body["data"]["profile"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );
}

#[test]
fn plan_describe_renders_target_sql_without_executing() {
    let output = altaika()
        .args([
            "plan",
            "describe",
            "local/public/events",
            "--target",
            "postgresql",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["target_dialect"], "postgresql");
    assert_eq!(
        body["data"]["canonical_sql"],
        "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'events'"
    );
}

#[test]
fn plan_describe_includes_registered_source_profile() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "plan",
            "describe",
            "local/events",
            "--target",
            "datafusion",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["profile"]["path"], "local/events");
    assert_eq!(body["data"]["profile"]["source_format"], "csv");
    assert_eq!(
        body["data"]["profile"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );
}

#[test]
fn plan_cat_includes_registered_source_profile() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args([
            "--csv",
            &registration,
            "plan",
            "cat",
            "local/events",
            "--target",
            "datafusion",
            "--columns",
            "id",
            "--limit",
            "10",
        ])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["data"]["profile"]["path"], "local/events");
    assert_eq!(
        body["data"]["profile"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );
}

#[test]
fn agent_schema_lists_agent_facing_commands() {
    let output = altaika()
        .args(["agent", "schema"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "agent_schema");
    assert_eq!(body["schema_version"], "1.0");
    assert_eq!(
        body["data"]["engines"],
        serde_json::json!(["datafusion", "altertable", "duckdb-beta"])
    );
    assert_eq!(
        body["data"]["dialects"],
        serde_json::json!([
            "duckdb",
            "ducklake",
            "datafusion",
            "postgresql",
            "snowflake",
            "databricks"
        ])
    );
    assert_eq!(
        body["data"]["output_formats"],
        serde_json::json!(["json", "ndjson"])
    );
    assert_eq!(
        body["data"]["profile_fields"],
        serde_json::json!([
            "path",
            "estimated_rows",
            "source_uri",
            "source_format",
            "source_size_bytes",
            "partition_keys",
            "clustering_keys",
            "sort_keys",
            "engine_hints",
            "columns"
        ])
    );
    assert_eq!(
        body["data"]["row_meta_fields"],
        serde_json::json!(["row_count", "limit", "columns", "profile"])
    );
    assert_eq!(
        body["data"]["filter_syntax"],
        serde_json::json!([
            "column:=value",
            "column:>value",
            "column:>=value",
            "column:<value",
            "column:<=value"
        ])
    );
    let commands = body["data"]["commands"].as_array().expect("commands array");
    assert!(
        commands
            .iter()
            .any(|command| command["name"] == "plan cat"
                && command["example"] == "altaika plan cat local/events --target snowflake --columns id,event_name --limit 10")
    );
    assert!(
        commands
            .iter()
            .any(|command| command["name"] == "snapshot" && command["output_kind"] == "snapshot")
    );
    assert!(commands.iter().any(|command| command["name"] == "cat"
        && command["help"] == "altaika cat --help"
        && command["skill"] == "docs/skills/commands/cat.md"));
    assert!(commands.iter().any(|command| command["name"] == "plan cat"
        && command["help"] == "altaika plan cat --help"
        && command["skill"] == "docs/skills/commands/plan-cat.md"));
    assert!(
        commands
            .iter()
            .any(|command| command["name"] == "agent issue-template"
                && command["help"] == "altaika agent issue-template --help"
                && command["output_kind"] == "issue_template")
    );
}

#[test]
fn agent_schema_compact_keeps_command_discovery_small() {
    let output = altaika()
        .args(["agent", "schema", "--compact"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "agent_schema");
    assert_eq!(body["compact"], true);
    assert_eq!(
        body["data"]["engines"],
        serde_json::json!(["datafusion", "altertable", "duckdb-beta"])
    );
    let commands = body["data"]["commands"].as_array().expect("commands array");
    let cat = commands
        .iter()
        .find(|command| command["name"] == "cat")
        .expect("cat command");
    assert_eq!(cat["help"], "altaika cat --help");
    assert_eq!(cat["skill"], "docs/skills/commands/cat.md");
    assert!(cat.get("purpose").is_none());
    assert!(cat.get("example").is_none());
}

#[test]
fn skills_list_returns_command_skill_index() {
    let output = altaika()
        .args(["skills", "list"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "skills");
    assert_eq!(body["schema_version"], "1.0");
    let skills = body["data"].as_array().expect("skills array");
    assert!(skills.iter().any(|skill| skill["name"] == "cat"
        && skill["command"] == "altaika cat --help"
        && skill["path"] == "docs/skills/commands/cat.md"));
    assert!(
        skills
            .iter()
            .any(|skill| skill["name"] == "agent issue-template"
                && skill["command"] == "altaika agent issue-template --help"
                && skill["path"] == "docs/skills/commands/agent-issue-template.md")
    );
    assert!(skills.iter().any(|skill| skill["name"] == "skills list"
        && skill["command"] == "altaika skills list --help"
        && skill["path"] == "docs/skills/commands/skills-list.md"));
}

#[test]
fn agent_issue_template_returns_structured_github_body() {
    let output = altaika()
        .args(["agent", "issue-template"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "issue_template");
    assert_eq!(body["schema_version"], "1.0");
    assert_eq!(body["data"]["repository"], "altertable-ai/altaika");
    assert_eq!(body["data"]["title"], "CLI behavior needs review");
    assert!(
        body["data"]["body"]
            .as_str()
            .expect("issue body")
            .contains("## Command")
    );
}

#[test]
fn agent_schema_help_and_skill_pointers_do_not_drift() {
    let output = altaika()
        .args(["agent", "schema"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    let commands = body["data"]["commands"].as_array().expect("commands array");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    for command in commands {
        let name = command["name"].as_str().expect("command name");
        let help = command["help"].as_str().expect("help command");
        let skill = command["skill"].as_str().expect("skill path");

        assert!(
            repo_root.join(skill).exists(),
            "{name} references missing skill file {skill}"
        );

        let args = help
            .strip_prefix("altaika ")
            .expect("help command starts with altaika")
            .split_whitespace()
            .collect::<Vec<_>>();
        let output = altaika().args(args).output().expect("run help command");
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).expect("utf8 help");
        assert!(
            stdout.contains("Command skill:"),
            "{name} help did not render command skill:\n{stdout}"
        );
    }
}

#[test]
fn help_includes_command_skills_from_markdown() {
    let cases = [
        (vec!["--help"], "Command skill: altaika"),
        (
            vec!["completions", "--help"],
            "Command skill: altaika completions",
        ),
        (vec!["ls", "--help"], "Command skill: altaika ls"),
        (
            vec!["describe", "--help"],
            "Command skill: altaika describe",
        ),
        (vec!["cat", "--help"], "Command skill: altaika cat"),
        (
            vec!["snapshot", "--help"],
            "Command skill: altaika snapshot",
        ),
        (vec!["auth", "--help"], "Command skill: altaika auth"),
        (vec!["plan", "--help"], "Command skill: altaika plan"),
        (
            vec!["plan", "ls", "--help"],
            "Command skill: altaika plan ls",
        ),
        (
            vec!["plan", "describe", "--help"],
            "Command skill: altaika plan describe",
        ),
        (
            vec!["plan", "cat", "--help"],
            "Command skill: altaika plan cat",
        ),
        (vec!["agent", "--help"], "Command skill: altaika agent"),
        (
            vec!["agent", "schema", "--help"],
            "Command skill: altaika agent schema",
        ),
        (
            vec!["agent", "issue-template", "--help"],
            "Command skill: altaika agent issue-template",
        ),
        (vec!["skills", "--help"], "Command skill: altaika skills"),
        (
            vec!["skills", "list", "--help"],
            "Command skill: altaika skills list",
        ),
        (vec!["sql", "--help"], "Command skill: altaika sql"),
    ];

    for (args, expected) in cases {
        let output = altaika().args(args).output().expect("run altaika");
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).expect("utf8 help");
        assert!(
            stdout.contains(expected),
            "expected help to contain {expected:?}, got:\n{stdout}"
        );
    }
}

#[test]
fn completions_generates_zsh_script() {
    let output = altaika()
        .args(["completions", "zsh"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8 completions");
    assert!(stdout.contains("#compdef altaika"));
    assert!(stdout.contains("_altaika()"));
    assert!(stdout.contains("'cat:Read bounded rows from a table'"));
}

#[test]
fn help_includes_actionable_command_and_option_descriptions() {
    let root = altaika().arg("--help").output().expect("run altaika");
    assert!(root.status.success(), "{root:?}");
    let root = String::from_utf8(root.stdout).expect("utf8 help");
    assert!(root.contains("completions  Generate shell completions"));
    assert!(root.contains("ls           List catalogs, schemas, tables, or columns"));
    assert!(root.contains("cat          Read bounded rows from a table"));
    assert!(root.contains("--engine <ENGINE>"));
    assert!(root.contains("Data engine to use"));

    let cat = altaika()
        .args(["cat", "--help"])
        .output()
        .expect("run altaika");
    assert!(cat.status.success(), "{cat:?}");
    let cat = String::from_utf8(cat.stdout).expect("utf8 help");
    assert!(cat.contains("<PATH>"));
    assert!(cat.contains("Table path as source/schema/table"));
    assert!(cat.contains("--columns <COLUMNS>"));
    assert!(cat.contains("Comma-separated columns to return"));
    assert!(cat.contains("--filter <FILTERS>"));
    assert!(cat.contains("Filter predicate like column:=value"));
    assert!(
        !cat.contains("          \n\n"),
        "help should not render empty detail blocks:\n{cat}"
    );
}

#[test]
fn describe_reads_registered_csv_schema() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args(["--csv", &registration, "describe", "local/public/events"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "schema");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["profile"]["path"], "local/public/events");
    assert_eq!(body["profile"]["estimated_rows"], serde_json::Value::Null);
    assert_eq!(body["profile"]["source_uri"], csv_path);
    assert_eq!(body["profile"]["source_format"], "csv");
    assert_eq!(
        body["profile"]["source_size_bytes"],
        fs::metadata(&csv_path).expect("csv metadata").len()
    );
    assert_eq!(body["profile"]["partition_keys"], serde_json::json!([]));
    assert_eq!(body["profile"]["engine_hints"], serde_json::json!([]));
    assert_eq!(
        body["data"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );
}

#[test]
fn describe_uses_registration_format_for_extensionless_csv() {
    let csv_path = temp_extensionless_file("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args(["--csv", &registration, "describe", "local/public/events"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["profile"]["source_uri"], csv_path);
    assert_eq!(body["profile"]["source_format"], "csv");
}

#[test]
fn ls_lists_registered_csv_tables() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args(["--csv", &registration, "ls", "local/public"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "listing");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(
        body["data"],
        serde_json::json!([{ "table_name": "events" }])
    );
}

#[test]
fn ls_long_includes_registered_table_profiles() {
    let csv_path = temp_csv("events", "id,event_name\n1,signup\n2,login\n");
    let registration = format!("public.events={csv_path}");

    let output = altaika()
        .args(["--csv", &registration, "ls", "--long", "local/public"])
        .output()
        .expect("run altaika");

    assert!(output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(body["kind"], "listing");
    assert_eq!(body["engine"], "datafusion");
    assert_eq!(body["data"][0]["table_name"], "events");
    assert_eq!(body["data"][0]["profile"]["path"], "local/public/events");
    assert_eq!(body["data"][0]["profile"]["estimated_rows"], Value::Null);
    assert_eq!(
        body["data"][0]["profile"]["columns"],
        serde_json::json!([
            {"name": "id", "data_type": "Int64", "nullable": true},
            {"name": "event_name", "data_type": "Utf8", "nullable": true}
        ])
    );
}

#[test]
fn unsupported_engine_returns_structured_error() {
    let output = altaika()
        .args(["--engine", "snowflake", "sql", "SELECT 1"])
        .output()
        .expect("run altaika");

    assert!(!output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stderr).expect("json stderr");
    assert_eq!(body["error_code"], "unsupported_operation");
    assert_eq!(body["exit_code"], 6);
}

#[test]
fn altertable_engine_requires_runtime_user() {
    let output = altaika()
        .args(["--engine", "altertable", "sql", "SELECT 1"])
        .env_remove("ALTERTABLE_USER")
        .output()
        .expect("run altaika");

    assert!(!output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stderr).expect("json stderr");
    assert_eq!(body["error_code"], "invalid_argument");
    assert_eq!(body["exit_code"], 2);
}

#[test]
fn parse_errors_return_structured_json() {
    let output = altaika().arg("cat").output().expect("run altaika");

    assert!(!output.status.success(), "{output:?}");

    let body: Value = serde_json::from_slice(&output.stderr).expect("json stderr");
    assert_eq!(body["error_code"], "invalid_argument");
    assert_eq!(body["exit_code"], 2);
}
