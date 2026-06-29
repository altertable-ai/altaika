use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Output, Stdio};

use serde_json::Value;

fn altaika() -> Command {
    Command::new(env!("CARGO_BIN_EXE_altaika"))
}

fn temp_dir(name: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "altaika-{name}-{}-{}",
        std::process::id(),
        unix_nanos()
    ));
    fs::create_dir_all(&path).expect("create temp dir");
    path.to_string_lossy().into_owned()
}

fn temp_path(name: &str, extension: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "altaika-{name}-{}-{}.{}",
            std::process::id(),
            unix_nanos(),
            extension
        ))
        .to_string_lossy()
        .into_owned()
}

fn temp_csv(name: &str, contents: &str) -> String {
    let path = temp_path(name, "csv");
    fs::write(&path, contents).expect("write csv");
    path
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos()
}

fn run(args: &[&str]) -> Output {
    altaika().args(args).output().expect("run altaika")
}

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn unused_local_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind local port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn stdout_json(output: &Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("json stdout")
}

fn stderr_json(output: &Output) -> Value {
    assert!(!output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stderr).expect("json stderr")
}

fn assert_agent_envelope(body: &Value, skill: &str, mode: &str) {
    assert_eq!(body["schema_version"], "1.0");
    assert_eq!(body["skill"], skill);
    assert_eq!(body["mode"], mode);
    assert!(body["stats"].is_object(), "{body}");
}

#[test]
fn explain_uses_local_file_stats_for_csv() {
    let csv = temp_csv("route-events", "id,event_name\n1,signup\n2,login\n");

    let body = stdout_json(&run(&["explain", &format!("csv://{csv}")]));

    assert_eq!(body["kind"], "explain");
    assert_agent_envelope(&body, "routing_explain", "local");
    assert_eq!(body["data"]["source_scheme"], "csv");
    assert_eq!(body["data"]["plan"]["fetch_engine"], "duckdb_scanner");
    assert_eq!(body["data"]["plan"]["execution"], "local");
    assert_eq!(body["data"]["plan"]["supported_now"], true);
    assert_eq!(
        body["data"]["decision"]["recommendation"],
        "local_workspace"
    );
    assert_eq!(body["data"]["signals"]["observed_file_size_bytes"], 31);
    assert_eq!(body["stats"]["operation"]["name"], "explain");
    assert_eq!(body["data"]["planned_operation"]["name"], "pull");
    assert_eq!(
        body["data"]["planned_permission"]["approval_required"],
        true
    );
}

#[test]
fn plan_and_route_remain_explain_aliases() {
    let csv = temp_csv("route-alias-events", "id,event_name\n1,signup\n");

    let plan = stdout_json(&run(&["plan", &format!("csv://{csv}")]));
    let route = stdout_json(&run(&["route", &format!("csv://{csv}")]));

    assert_eq!(plan["kind"], "explain");
    assert_eq!(route["kind"], "explain");
    assert_agent_envelope(&plan, "routing_explain", "local");
    assert_agent_envelope(&route, "routing_explain", "local");
}

#[test]
fn explain_recommends_explicit_pull_for_remote_platforms() {
    let body = stdout_json(&run(&[
        "explain",
        "bigquery://project.dataset.events",
        "--estimated-bytes",
        "1048576",
        "--estimated-rows",
        "5000",
        "--source-latency-ms",
        "300",
    ]));

    assert_eq!(body["kind"], "explain");
    assert_agent_envelope(&body, "routing_explain", "remote");
    assert_eq!(body["data"]["source_scheme"], "bigquery");
    let expected_fetch_engine = if cfg!(feature = "bigquery-adbc") {
        "bigquery_adbc_arrow_stream"
    } else {
        "datafusion_connector_or_source_connector"
    };
    assert_eq!(body["data"]["plan"]["fetch_engine"], expected_fetch_engine);
    assert_eq!(
        body["data"]["plan"]["supported_now"],
        cfg!(feature = "bigquery-adbc")
    );
    assert_eq!(
        body["data"]["decision"]["recommendation"],
        "explicit_pull_then_local"
    );
    assert_eq!(
        body["data"]["signals"]["estimated_data_size_bytes"],
        1_048_576
    );
    assert_eq!(body["data"]["signals"]["latency_class"], "high");
    assert_eq!(
        body["data"]["boundaries"]["datafusion"]["required_for_this_plan"],
        !cfg!(feature = "bigquery-adbc")
    );
}

#[test]
fn bigquery_pull_reports_connector_setup_hints() {
    let workspace = temp_dir("bigquery-setup");

    let body = stderr_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "allow",
        "pull",
        "bigquery://bigquery-public-data.usa_names.usa_1910_2013",
        "--connector",
        "adbc",
        "--table",
        "usa_names",
        "--local-target",
        "ducklake",
        "--limit",
        "5",
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "connector_setup", "remote");
    assert_eq!(body["stats"]["required_feature"], "bigquery-adbc");
    assert_eq!(body["stats"]["required_driver"], "adbc_driver_bigquery");
    assert_eq!(
        body["stats"]["source_table"],
        "bigquery-public-data.usa_names.usa_1910_2013"
    );
    assert_eq!(body["stats"]["source_project"], "bigquery-public-data");
    assert_eq!(body["stats"]["source_dataset"], "usa_names");
    assert_eq!(body["stats"]["source_table_name"], "usa_1910_2013");
    assert_eq!(
        body["stats"]["source_sql"],
        "SELECT * FROM `bigquery-public-data.usa_names.usa_1910_2013` LIMIT 5"
    );
    assert!(body["stats"]["next_commands"].as_array().is_some());
}

#[test]
fn permission_blocks_local_writes_by_default() {
    let workspace = temp_dir("permission");

    let body = stderr_json(&run(&[
        "--workspace",
        &workspace,
        "init",
        "--local-target",
        "duckdb",
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "approval_required", "local");
    assert_eq!(body["stats"]["permission"]["mode"], "permission");
    assert_eq!(
        body["stats"]["permission"]["operation"]["class"],
        "local_write"
    );
}

#[test]
fn init_creates_duckdb_workspace_with_auto_permission() {
    let workspace = temp_dir("workspace");
    let database = format!("{workspace}/workspace.duckdb");

    let body = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "init",
        "--local-target",
        "duckdb",
    ]));

    assert_eq!(body["kind"], "workspace");
    assert_agent_envelope(&body, "workspace_init", "local");
    assert_eq!(body["data"]["target"], "duckdb");
    assert_eq!(body["data"]["database"], database);
    assert_eq!(body["stats"]["permission"]["mode"], "auto");
    assert!(fs::metadata(database).expect("duckdb database").is_file());
}

#[test]
fn pull_csv_to_duckdb_then_inspect_describe_and_show() {
    let workspace = temp_dir("pull");
    let csv = temp_csv("events", "id,event_name\n1,signup\n2,login\n3,signup\n");

    let pull = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "pull",
        &format!("csv://{csv}"),
        "--table",
        "events",
        "--local-target",
        "duckdb",
        "--limit",
        "2",
    ]));

    assert_eq!(pull["kind"], "pull");
    assert_agent_envelope(&pull, "source_pull", "local");
    assert_eq!(pull["data"]["row_count"], 2);
    assert_eq!(pull["data"]["target"], "duckdb");

    let list = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "ls",
        "--engine",
        "duckdb",
    ]));
    assert_eq!(list["kind"], "ls");
    assert_agent_envelope(&list, "catalog_list", "local");
    assert_eq!(list["data"]["rows"][0]["table_name"], "events");

    let describe = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "describe",
        "--engine",
        "duckdb",
        "events",
    ]));
    assert_eq!(describe["kind"], "describe");
    assert_agent_envelope(&describe, "table_describe", "local");
    assert_eq!(describe["data"]["rows"][0]["column_name"], "id");

    let show = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "show",
        "--engine",
        "duckdb",
        "events",
        "--limit",
        "1",
    ]));
    assert_eq!(show["kind"], "show");
    assert_agent_envelope(&show, "table_preview", "local");
    assert_eq!(show["data"]["rows_returned"], 1);
    assert_eq!(show["data"]["rows"][0]["event_name"], "signup");
}

#[test]
fn query_runs_sql_locally_and_records_manifest() {
    let workspace = temp_dir("query");

    let create = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "query",
        "--engine",
        "duckdb",
        "CREATE OR REPLACE TABLE events AS SELECT 1 AS id UNION ALL SELECT 2 AS id",
    ]));
    assert_eq!(create["kind"], "query");
    assert_agent_envelope(&create, "local_query", "local");
    assert_eq!(create["data"]["rows_returned"], 0);

    let select = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "query",
        "--engine",
        "duckdb",
        "SELECT count(*) AS rows FROM events",
    ]));
    assert_eq!(select["kind"], "query");
    assert_agent_envelope(&select, "local_query", "local");
    assert_eq!(select["data"]["rows"][0]["rows"], 2);
    assert_eq!(select["stats"]["operation"]["name"], "query");

    let inspect = stdout_json(&run(&["--workspace", &workspace, "inspect"]));
    assert_eq!(inspect["kind"], "inspect");
    assert_agent_envelope(&inspect, "workspace_status", "local");
    assert!(inspect["data"]["run_count"].as_u64().unwrap_or_default() >= 2);
}

#[test]
fn run_alias_still_executes_query() {
    let workspace = temp_dir("run-alias");

    let body = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "run",
        "--engine",
        "duckdb",
        "SELECT 1 AS ok",
    ]));

    assert_eq!(body["kind"], "query");
    assert_agent_envelope(&body, "local_query", "local");
    assert_eq!(body["data"]["rows"][0]["ok"], 1);
}

#[test]
fn query_dry_run_explains_without_writing_manifest() {
    let workspace = temp_dir("dry-run");
    let _ = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "init",
        "--local-target",
        "duckdb",
    ]));

    let body = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "query",
        "--engine",
        "duckdb",
        "--dry-run",
        "SELECT 1 AS ok",
    ]));

    assert_eq!(body["kind"], "query");
    assert_agent_envelope(&body, "query_explain", "local");
    assert_eq!(body["data"]["dry_run"], true);
    assert_eq!(body["data"]["manifest_path"], Value::Null);
    assert_eq!(body["stats"]["manifest_written"], false);
}

#[test]
fn query_dry_run_rejects_statement_sequences_before_execution() {
    let workspace = temp_dir("dry-run-sequence");
    let output_path = temp_path("dry-run-copy", "csv");

    let body = stderr_json(&run(&[
        "--workspace",
        &workspace,
        "query",
        "--engine",
        "duckdb",
        "--dry-run",
        &format!("SELECT 1; COPY (SELECT 42 AS x) TO '{output_path}'"),
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "error_report", "local");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one SQL statement")
    );
    assert!(fs::metadata(output_path).is_err());
}

#[test]
fn query_dry_run_rejects_explain_analyze_before_execution() {
    let workspace = temp_dir("dry-run-analyze");
    let output_path = temp_path("dry-run-analyze-copy", "csv");

    let body = stderr_json(&run(&[
        "--workspace",
        &workspace,
        "query",
        "--engine",
        "duckdb",
        "--dry-run",
        &format!("EXPLAIN /* agent */ ANALYZE COPY (SELECT 42 AS x) TO '{output_path}'"),
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "error_report", "local");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("EXPLAIN ANALYZE")
    );
    assert!(fs::metadata(output_path).is_err());
}

#[test]
fn query_returns_bounded_rows_for_agents() {
    let workspace = temp_dir("query-max-rows");

    let _ = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "query",
        "--engine",
        "duckdb",
        "CREATE OR REPLACE TABLE bounded AS SELECT * FROM (VALUES (1), (2), (3)) AS t(id)",
    ]));

    let body = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "query",
        "--engine",
        "duckdb",
        "--max-rows",
        "2",
        "SELECT id FROM bounded ORDER BY id",
    ]));

    assert_eq!(body["kind"], "query");
    assert_agent_envelope(&body, "local_query", "local");
    assert_eq!(body["stats"]["row_limit"], 2);
    assert_eq!(body["stats"]["rows_returned"], 2);
    assert_eq!(body["stats"]["rows_truncated"], true);
    assert_eq!(body["data"]["rows"].as_array().map_or(0, Vec::len), 2);
    assert_eq!(body["data"]["rows"][0]["id"], 1);
    assert_eq!(body["data"]["rows"][1]["id"], 2);
}

#[test]
fn remote_query_requires_allow_permission_before_network_access() {
    let body = stderr_json(&run(&[
        "query",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "--remote",
        "quack://localhost:6544",
        "SELECT 1",
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "approval_required", "remote");
    assert_eq!(
        body["stats"]["permission"]["operation"]["class"],
        "remote_read"
    );
}

#[test]
fn quack_query_redacts_token_from_remote_errors() {
    let token = "ABCDSECRET";

    let body = stderr_json(&run(&[
        "--permission",
        "allow",
        "quack",
        "query",
        "--remote",
        "quack://localhost:1",
        "--quack-token",
        token,
        "SELECT 1",
    ]));

    let error = body["error"].as_str().unwrap_or_default();
    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "error_report", "remote");
    assert!(!error.contains(token), "{error}");
    assert!(!error.contains("ABCD"), "{error}");
    assert!(error.contains("[redacted]"), "{error}");
}

#[test]
#[allow(clippy::too_many_lines)]
fn quack_serve_and_remote_inspection_commands_work() {
    let workspace = temp_dir("quack-serve");
    let token = "altaika-test";
    let port = unused_local_port();
    let remote_uri = format!("quack:localhost:{port}");
    let remote_arg = format!("quack://localhost:{port}");

    let _ = stdout_json(&run(&[
        "--workspace",
        &workspace,
        "--permission",
        "auto",
        "query",
        "--engine",
        "duckdb",
        "CREATE OR REPLACE TABLE remote_names AS SELECT * FROM (VALUES ('AK', 1910, 'F', 'Mary', 14), ('AK', 1910, 'F', 'Annie', 12), ('AK', 1910, 'M', 'John', 8)) AS t(state, year, gender, name, number)",
    ]));

    let mut child = altaika()
        .args([
            "--workspace",
            &workspace,
            "--permission",
            "allow",
            "quack",
            "serve",
            "--engine",
            "duckdb",
            "--remote",
            &remote_uri,
            "--quack-token",
            token,
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn quack serve");
    let stdout = child.stdout.take().expect("quack serve stdout");
    let _guard = ChildGuard { child };
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("quack readiness line");
    let ready: Value = serde_json::from_str(&line).expect("quack readiness json");

    assert_eq!(ready["kind"], "quack_serve");
    assert_agent_envelope(&ready, "quack_server", "remote");
    assert_eq!(ready["data"]["remote"]["uri"], remote_uri);
    assert_eq!(ready["data"]["rows"][0]["auth_token"], "[redacted]");

    let count = stdout_json(&run(&[
        "--permission",
        "allow",
        "quack",
        "query",
        "--remote",
        &remote_arg,
        "--quack-token",
        token,
        "SELECT count(*) AS rows, sum(number) AS total FROM remote_names",
    ]));
    assert_eq!(count["kind"], "quack_query");
    assert_agent_envelope(&count, "remote_query", "remote");
    assert_eq!(count["data"]["rows"][0]["rows"], 3);
    assert_eq!(count["data"]["rows"][0]["total"], "34");
    assert_eq!(count["stats"]["manifest_written"], false);

    let list = stdout_json(&run(&[
        "--permission",
        "allow",
        "ls",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "--remote",
        &remote_arg,
        "--quack-token",
        token,
    ]));
    assert_eq!(list["data"]["rows"][0]["table_name"], "remote_names");

    let describe = stdout_json(&run(&[
        "--permission",
        "allow",
        "describe",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "--remote",
        &remote_arg,
        "--quack-token",
        token,
        "remote_names",
    ]));
    assert_eq!(describe["data"]["rows_returned"], 5);

    let show = stdout_json(&run(&[
        "--permission",
        "allow",
        "show",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "--remote",
        &remote_arg,
        "--quack-token",
        token,
        "remote_names",
        "--limit",
        "1",
    ]));
    assert_eq!(show["data"]["rows"][0]["name"], "Mary");
}

#[test]
fn bigquery_pull_reports_driver_auth_and_parity_setup() {
    let body = stderr_json(&run(&[
        "--permission",
        "allow",
        "pull",
        "bigquery://bigquery-public-data.usa_names.usa_1910_2013",
        "--connector",
        "adbc",
        "--table",
        "usa_names",
        "--local-target",
        "ducklake",
        "--limit",
        "10",
        "--max-bytes-billed",
        "100000000",
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "connector_setup", "remote");
    assert!(
        !body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not implemented")
    );
    assert_eq!(body["stats"]["connector"], "bigquery");
    assert_eq!(
        body["stats"]["source_table"],
        "bigquery-public-data.usa_names.usa_1910_2013"
    );
    assert_eq!(
        body["stats"]["source_sql"],
        "SELECT * FROM `bigquery-public-data.usa_names.usa_1910_2013` LIMIT 10"
    );
    assert!(body["stats"]["auth"].is_object());
    assert!(body["stats"]["driver"].is_object());
    assert_eq!(
        body["stats"]["migration_goal"]["parity_checks"][0],
        "SELECT count(*) AS rows FROM usa_names"
    );
    assert_eq!(
        body["stats"]["migration_goal"]["parity_checks"][1],
        "DESCRIBE usa_names"
    );
}

#[test]
fn invalid_remote_query_args_are_json() {
    let body = stderr_json(&run(&[
        "--permission",
        "allow",
        "query",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "SELECT 1",
    ]));

    assert_eq!(body["kind"], "error");
    assert_agent_envelope(&body, "error_report", "remote");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("--remote")
    );
}

#[test]
fn doctor_reports_embedded_duckdb_runtime() {
    let body = stdout_json(&run(&["doctor"]));

    assert_eq!(body["kind"], "doctor");
    assert_agent_envelope(&body, "environment_doctor", "local");
    assert_eq!(
        body["data"]["checks"]["duckdb_runtime"]["client"],
        "duckdb-rs"
    );
    assert_eq!(
        body["data"]["checks"]["duckdb_runtime"]["version"]["ok"],
        true
    );
    assert_eq!(
        body["data"]["checks"]["duckdb_runtime"]["extensions"]["quack_functions"]["ok"],
        true
    );
}

#[test]
fn skills_install_dry_run_is_read_only() {
    let target = temp_dir("skills");

    let body = stdout_json(&run(&[
        "skills",
        "install",
        "--target-dir",
        &target,
        "--dry-run",
    ]));

    assert_eq!(body["kind"], "skills_install");
    assert_agent_envelope(&body, "skills_install", "local");
    assert_eq!(body["data"]["dry_run"], true);
    assert_eq!(body["data"]["files_written"], 0);
}

#[test]
#[ignore = "requires a local Quack endpoint"]
fn quack_remote_smoke() {
    let remote = std::env::var("ALTAIKA_IT_QUACK_URL")
        .expect("set ALTAIKA_IT_QUACK_URL=quack://127.0.0.1:6544");

    let body = stdout_json(&run(&[
        "--permission",
        "allow",
        "query",
        "--engine",
        "duckdb",
        "--mode",
        "remote",
        "--remote",
        &remote,
        "SELECT 1 AS ok",
    ]));

    assert_eq!(body["kind"], "query");
    assert_agent_envelope(&body, "remote_query", "remote");
    assert_eq!(body["data"]["rows"][0]["ok"], 1);
}
