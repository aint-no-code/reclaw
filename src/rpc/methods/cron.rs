use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::{cron_schedule::compute_next_run_ms, state::SharedState},
    domain::models::{CronJobPatch, CronJobRecord, CronPayload, CronSchedule},
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronListParams {
    #[serde(default)]
    include_disabled: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronStatusParams {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronAddParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    schedule: CronSchedule,
    payload: CronPayload,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronUpdateParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    patch: CronPatchInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronPatchInput {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    schedule: Option<CronSchedule>,
    #[serde(default)]
    payload: Option<CronPayload>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    next_run_ms: Option<Option<u64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronIdParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CronRunsParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn handle_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronListParams = parse_optional_params("cron.list", params)?;
    let include_disabled = parsed.include_disabled.unwrap_or(true);
    let mut jobs = state.list_cron_jobs().await.map_err(map_domain_error)?;
    if !include_disabled {
        jobs.retain(|job| job.enabled);
    }

    if let Some(limit) = parsed.limit {
        jobs.truncate(limit);
    }

    Ok(json!({
        "jobs": jobs,
        "count": jobs.len(),
    }))
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: CronStatusParams = parse_optional_params("cron.status", params)?;
    state.cron_status().await.map_err(map_domain_error)
}

pub async fn handle_add(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronAddParams = parse_required_params("cron.add", params)?;
    validate_schedule(&parsed.schedule)?;

    let now = now_unix_ms();
    let id = parsed
        .id
        .and_then(trim_non_empty)
        .unwrap_or_else(|| format!("job-{}", uuid::Uuid::new_v4()));

    let name = parsed
        .name
        .and_then(trim_non_empty)
        .unwrap_or_else(|| format!("Cron {id}"));

    let next_run_ms = if parsed.enabled {
        compute_next_run_ms(&parsed.schedule, now).map_err(invalid_cron_error)?
    } else {
        None
    };

    let job = CronJobRecord {
        id,
        name,
        enabled: parsed.enabled,
        schedule: parsed.schedule,
        payload: parsed.payload,
        metadata: parsed.metadata.unwrap_or_else(|| json!({})),
        created_at_ms: now,
        updated_at_ms: now,
        last_run_ms: None,
        next_run_ms,
    };

    state.add_cron_job(&job).await.map_err(map_domain_error)?;
    Ok(json!(job))
}

pub async fn handle_update(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronUpdateParams = parse_required_params("cron.update", params)?;
    let id = resolve_cron_id(parsed.id, parsed.job_id, "cron.update")?;

    if let Some(schedule) = parsed.patch.schedule.as_ref() {
        validate_schedule(schedule)?;
    }

    let next_run_ms = if let Some(next) = parsed.patch.next_run_ms {
        Some(next)
    } else if let Some(schedule) = parsed.patch.schedule.as_ref() {
        let computed = compute_next_run_ms(schedule, now_unix_ms()).map_err(invalid_cron_error)?;
        Some(computed)
    } else {
        None
    };

    let patch = CronJobPatch {
        name: parsed.patch.name.and_then(trim_non_empty),
        enabled: parsed.patch.enabled,
        schedule: parsed.patch.schedule,
        payload: parsed.patch.payload,
        metadata: parsed.patch.metadata,
        next_run_ms,
    };

    let updated = state
        .update_cron_job(&id, patch)
        .await
        .map_err(map_domain_error)?;

    Ok(json!(updated))
}

pub async fn handle_remove(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronIdParams = parse_required_params("cron.remove", params)?;
    let id = resolve_cron_id(parsed.id, parsed.job_id, "cron.remove")?;

    let removed = state.remove_cron_job(&id).await.map_err(map_domain_error)?;
    Ok(json!({
        "ok": true,
        "id": id,
        "removed": removed,
    }))
}

pub async fn handle_run(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronIdParams = parse_required_params("cron.run", params)?;
    let id = resolve_cron_id(parsed.id, parsed.job_id, "cron.run")?;

    let run = state
        .run_cron_job_now(&id)
        .await
        .map_err(map_domain_error)?;
    Ok(json!(run))
}

pub async fn handle_runs(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: CronRunsParams = parse_optional_params("cron.runs", params)?;
    let job_id = parsed.id.or(parsed.job_id).and_then(trim_non_empty);
    let limit = parsed.limit.map(|value| value.clamp(1, 1_000));

    let runs = state
        .list_cron_runs(job_id.as_deref(), limit)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "scope": if job_id.is_some() { "job" } else { "all" },
        "jobId": job_id,
        "runs": runs,
        "count": runs.len(),
    }))
}

fn validate_schedule(schedule: &CronSchedule) -> Result<(), crate::protocol::ErrorShape> {
    if schedule.kind.trim().is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid cron schedule: kind is required",
        ));
    }

    let _ = compute_next_run_ms(schedule, now_unix_ms()).map_err(invalid_cron_error)?;
    Ok(())
}

fn invalid_cron_error(message: String) -> crate::protocol::ErrorShape {
    crate::protocol::ErrorShape::new(
        crate::protocol::ERROR_INVALID_REQUEST,
        format!("invalid cron schedule: {message}"),
    )
}

fn resolve_cron_id(
    id: Option<String>,
    job_id: Option<String>,
    method: &str,
) -> Result<String, crate::protocol::ErrorShape> {
    id.or(job_id).and_then(trim_non_empty).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: missing id"),
        )
    })
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

const fn default_true() -> bool {
    true
}
