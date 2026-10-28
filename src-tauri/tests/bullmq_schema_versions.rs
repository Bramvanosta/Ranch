//! CI-blocking test for the BullMQ schema decoder module.
//!
//! Walks every fixture under
//! `crates/bullmq-redis/src/bullmq_schema/fixtures/v5/` and `fixtures/v6/`,
//! decodes each via the matching version's `JobDecoder`, and asserts the
//! decoded `Job` matches the fixture's `expected` shape field-by-field.
//!
//! Adding a new BullMQ minor = drop new fixtures in `fixtures/vN/` (and
//! a `vN.rs` decoder if encoding drifts); this test picks them up
//! automatically, no test code change needed for new fixtures of an
//! existing version.
//!
//! The fixture count grows with every supported BullMQ minor; the test
//! count grows with it (we use a single dynamic test that asserts each
//! fixture; failure messages name the fixture file so a regression in
//! one decoder can be traced quickly).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{
    detect_schema, states, BullmqSchema, Job, JobDecoder, JobRef, JobState, QueueRef, SchemaError,
    V5Decoder, V6Decoder,
};

/// Fixture root, relative to the crate root (Cargo runs tests with CWD
/// = crate root).
const FIXTURES_ROOT: &str = "crates/bullmq-redis/src/bullmq_schema/fixtures";

#[test]
fn every_fixture_decodes_against_its_version() {
    let cases = collect_fixtures();
    assert!(
        !cases.is_empty(),
        "no fixtures discovered under {FIXTURES_ROOT}"
    );

    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        if let Err(e) = run_case(case) {
            failures.push(format!("{}: {}", case.path.display(), e));
        }
    }

    assert!(
        failures.is_empty(),
        "{} fixture(s) failed:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn detect_schema_round_trip_for_unknown_versions() {
    let mut meta = HashMap::new();
    meta.insert("version".to_string(), "7.0.0".to_string());
    let schema = detect_schema(&meta);
    assert_eq!(
        schema,
        BullmqSchema::Unknown {
            detected_version: Some("7.0.0".to_string())
        }
    );

    let empty = HashMap::new();
    assert_eq!(
        detect_schema(&empty),
        BullmqSchema::Unknown {
            detected_version: None
        }
    );
}

#[test]
fn fixture_count_meets_minimum_per_version() {
    // Ship at least 3 fixtures per version covering the documented
    // states (waiting / failed / completed for v5; delayed / flow_child
    // / custom_prefix for v6). This guards against accidentally deleting
    // a fixture without realizing the floor was breached.
    let v5_count = list_jsons(&fixtures_dir().join("v5")).len();
    let v6_count = list_jsons(&fixtures_dir().join("v6")).len();
    assert!(v5_count >= 3, "v5 fixture count = {v5_count} (need ≥ 3)");
    assert!(v6_count >= 3, "v6 fixture count = {v6_count} (need ≥ 3)");
}

// ── Internals ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct FixtureCase {
    path: PathBuf,
    version: BullmqSchema,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_ROOT)
}

fn collect_fixtures() -> Vec<FixtureCase> {
    let root = fixtures_dir();
    let mut out = Vec::new();
    for (subdir, version) in [("v5", BullmqSchema::V5), ("v6", BullmqSchema::V6)] {
        let dir = root.join(subdir);
        for path in list_jsons(&dir) {
            out.push(FixtureCase {
                path,
                version: version.clone(),
            });
        }
    }
    out
}

fn list_jsons(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect()
}

fn run_case(case: &FixtureCase) -> Result<(), String> {
    let raw = fs::read_to_string(&case.path).map_err(|e| format!("read failed: {e}"))?;
    let fixture: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("fixture JSON parse: {e}"))?;

    let jref = build_jref(&fixture)?;
    let fields = build_fields(&fixture)?;
    let job = decode_for(&case.version, jref, fields).map_err(|e| format!("decode: {e}"))?;

    assert_expectations(&job, &fixture["expected"])
}

fn build_jref(fixture: &serde_json::Value) -> Result<JobRef, String> {
    let qref = &fixture["queueRef"];
    let queue = QueueRef {
        prefix: qref["prefix"]
            .as_str()
            .ok_or("queueRef.prefix not a string")?
            .to_string(),
        name: qref["name"]
            .as_str()
            .ok_or("queueRef.name not a string")?
            .to_string(),
    };
    let id = fixture["jobId"]
        .as_str()
        .ok_or("jobId not a string")?
        .to_string();
    Ok(queue.job(id))
}

fn build_fields(fixture: &serde_json::Value) -> Result<HashMap<String, Vec<u8>>, String> {
    let raw = fixture["fields"]
        .as_object()
        .ok_or("fields not an object")?;
    let mut out = HashMap::with_capacity(raw.len());
    for (k, v) in raw {
        let s = v.as_str().ok_or_else(|| {
            format!("fields.{k} must be a string (BullMQ HGETALL is byte-strings)")
        })?;
        out.insert(k.clone(), s.as_bytes().to_vec());
    }
    Ok(out)
}

fn decode_for(
    schema: &BullmqSchema,
    jref: JobRef,
    fields: HashMap<String, Vec<u8>>,
) -> Result<Job, SchemaError> {
    match schema {
        BullmqSchema::V5 => V5Decoder.decode_job(jref, fields),
        BullmqSchema::V6 => V6Decoder.decode_job(jref, fields),
        BullmqSchema::Unknown { detected_version } => Err(SchemaError::SchemaNotRecognized {
            detected_version: detected_version.clone(),
        }),
    }
}

fn assert_expectations(job: &Job, expected: &serde_json::Value) -> Result<(), String> {
    let exp = expected.as_object().ok_or("expected must be an object")?;

    if let Some(v) = exp.get("name").and_then(|v| v.as_str()) {
        if job.name != v {
            return Err(format!("name mismatch: got {:?}, want {:?}", job.name, v));
        }
    }
    if let Some(v) = exp.get("state").and_then(|v| v.as_str()) {
        let want = match v {
            "waiting" => JobState::Waiting,
            "active" => JobState::Active,
            "completed" => JobState::Completed,
            "failed" => JobState::Failed,
            "delayed" => JobState::Delayed,
            "prioritized" => states::PRIORITIZED,
            "waitingChildren" => states::WAITING_CHILDREN,
            "stalled" => states::STALLED,
            "unknown" => states::UNKNOWN,
            other => return Err(format!("expected.state has unknown value {other:?}")),
        };
        if job.state != want {
            return Err(format!(
                "state mismatch: got {:?}, want {:?}",
                job.state, want
            ));
        }
    }
    if let Some(v) = exp.get("attemptsMade").and_then(|v| v.as_u64()) {
        if job.attempts_made as u64 != v {
            return Err(format!(
                "attemptsMade mismatch: got {}, want {v}",
                job.attempts_made
            ));
        }
    }
    if let Some(v) = exp.get("timestamp").and_then(|v| v.as_i64()) {
        if job.timestamp != v {
            return Err(format!(
                "timestamp mismatch: got {}, want {v}",
                job.timestamp
            ));
        }
    }
    if let Some(v) = exp.get("processedOn").and_then(|v| v.as_i64()) {
        if job.processed_on != Some(v) {
            return Err(format!(
                "processedOn mismatch: got {:?}, want Some({v})",
                job.processed_on
            ));
        }
    }
    if let Some(v) = exp.get("finishedOn").and_then(|v| v.as_i64()) {
        if job.finished_on != Some(v) {
            return Err(format!(
                "finishedOn mismatch: got {:?}, want Some({v})",
                job.finished_on
            ));
        }
    }
    if let Some(v) = exp.get("delay").and_then(|v| v.as_i64()) {
        if job.delay != Some(v) {
            return Err(format!(
                "delay mismatch: got {:?}, want Some({v})",
                job.delay
            ));
        }
    }
    if let Some(v) = exp.get("failedReason").and_then(|v| v.as_str()) {
        if job.failed_reason.as_deref() != Some(v) {
            return Err(format!(
                "failedReason mismatch: got {:?}, want {:?}",
                job.failed_reason, v
            ));
        }
    }
    if let Some(v) = exp.get("parentKey").and_then(|v| v.as_str()) {
        if job.parent_key.as_deref() != Some(v) {
            return Err(format!(
                "parentKey mismatch: got {:?}, want {:?}",
                job.parent_key, v
            ));
        }
    }
    if let Some(v) = exp.get("stacktraceCount").and_then(|v| v.as_u64()) {
        if job.stacktrace.len() as u64 != v {
            return Err(format!(
                "stacktraceCount mismatch: got {}, want {v}",
                job.stacktrace.len()
            ));
        }
    }
    // Wire shape: `data` / `opts` / `returnValue` are raw JSON
    // STRINGS. Parse them locally for the preview-style assertions so
    // fixtures stay readable.
    if let Some(preview) = exp.get("dataPreview").and_then(|v| v.as_object()) {
        let data: serde_json::Value = serde_json::from_str(&job.data)
            .map_err(|e| format!("decoded job.data not parseable: {e}"))?;
        for (k, want) in preview {
            let got = data
                .get(k)
                .ok_or_else(|| format!("data.{k} missing in decoded job (got {data})"))?;
            if got != want {
                return Err(format!("data.{k} mismatch: got {got}, want {want}"));
            }
        }
    }
    if let Some(preview) = exp.get("returnValuePreview").and_then(|v| v.as_object()) {
        let rv_str = job.return_value.as_ref().ok_or("returnValue missing")?;
        let rv: serde_json::Value = serde_json::from_str(rv_str)
            .map_err(|e| format!("decoded job.return_value not parseable: {e}"))?;
        for (k, want) in preview {
            let got = rv
                .get(k)
                .ok_or_else(|| format!("returnValue.{k} missing"))?;
            if got != want {
                return Err(format!("returnValue.{k} mismatch: got {got}, want {want}"));
            }
        }
    }
    Ok(())
}
