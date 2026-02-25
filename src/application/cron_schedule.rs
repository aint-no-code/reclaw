use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};

use crate::domain::models::CronSchedule;

pub fn compute_next_run_ms(schedule: &CronSchedule, from_ms: u64) -> Result<Option<u64>, String> {
    match schedule.kind.as_str() {
        "at" => {
            let at_text = schedule
                .at
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "schedule.at is required for kind=at".to_owned())?;
            let at_ms = parse_rfc3339_ms(at_text)?;
            if at_ms > from_ms {
                Ok(Some(at_ms))
            } else {
                Ok(None)
            }
        }
        "every" => {
            let every = schedule
                .every_ms
                .ok_or_else(|| "schedule.everyMs is required for kind=every".to_owned())?;
            if every == 0 {
                return Err("schedule.everyMs must be > 0".to_owned());
            }

            if let Some(anchor_ms) = schedule.anchor_ms {
                if from_ms < anchor_ms {
                    return Ok(Some(anchor_ms));
                }

                let elapsed = from_ms.saturating_sub(anchor_ms);
                let steps = elapsed / every;
                let next = anchor_ms
                    .saturating_add(steps.saturating_mul(every))
                    .saturating_add(every);
                Ok(Some(next))
            } else {
                Ok(Some(from_ms.saturating_add(every)))
            }
        }
        "cron" => {
            let expr = schedule
                .expr
                .as_deref()
                .map(str::trim)
                .filter(|expr| !expr.is_empty())
                .ok_or_else(|| "schedule.expr is required for kind=cron".to_owned())?;
            let next = compute_next_cron_time(expr, from_ms)?;
            Ok(Some(next))
        }
        "once" => Ok(None),
        other => Err(format!("unsupported schedule kind: {other}")),
    }
}

fn parse_rfc3339_ms(value: &str) -> Result<u64, String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| format!("invalid RFC3339 timestamp: {error}"))?;
    let millis = parsed.timestamp_millis();
    if millis < 0 {
        return Err("timestamp must be >= unix epoch".to_owned());
    }
    Ok(u64::try_from(millis).unwrap_or(u64::MAX))
}

fn compute_next_cron_time(expr: &str, from_ms: u64) -> Result<u64, String> {
    let parts = expr.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 5 && parts.len() != 6 {
        return Err("cron expression must contain 5 or 6 fields".to_owned());
    }

    let minute_index = if parts.len() == 6 { 1 } else { 0 };
    let hour_index = minute_index + 1;
    let dom_index = minute_index + 2;
    let month_index = minute_index + 3;
    let dow_index = minute_index + 4;

    if parts[hour_index] != "*"
        || parts[dom_index] != "*"
        || parts[month_index] != "*"
        || parts[dow_index] != "*"
    {
        return Err("only minute-based cron expressions are supported currently".to_owned());
    }

    let matcher = parse_minute_matcher(parts[minute_index])?;
    let start = DateTime::<Utc>::from_timestamp_millis(i64::try_from(from_ms).unwrap_or(i64::MAX))
        .ok_or_else(|| "invalid timestamp for cron computation".to_owned())?;

    for offset in 1..=(60 * 24 * 7) {
        let candidate = (start + ChronoDuration::minutes(offset))
            .with_second(0)
            .and_then(|value| value.with_nanosecond(0))
            .ok_or_else(|| "failed to normalize cron candidate".to_owned())?;

        if matcher.matches(candidate.minute()) {
            let ms = candidate.timestamp_millis();
            return Ok(u64::try_from(ms).unwrap_or(u64::MAX));
        }
    }

    Err("unable to compute next cron occurrence in 7-day search window".to_owned())
}

struct MinuteMatcher {
    mode: MinuteMatchMode,
}

impl MinuteMatcher {
    fn matches(&self, minute: u32) -> bool {
        match self.mode {
            MinuteMatchMode::Any => true,
            MinuteMatchMode::Every(step) => minute.is_multiple_of(step),
            MinuteMatchMode::Exact(value) => minute == value,
        }
    }
}

enum MinuteMatchMode {
    Any,
    Every(u32),
    Exact(u32),
}

fn parse_minute_matcher(field: &str) -> Result<MinuteMatcher, String> {
    let trimmed = field.trim();
    if trimmed == "*" {
        return Ok(MinuteMatcher {
            mode: MinuteMatchMode::Any,
        });
    }

    if let Some(step_text) = trimmed.strip_prefix("*/") {
        let step = step_text
            .parse::<u32>()
            .map_err(|_| "invalid minute step in cron expression".to_owned())?;
        if !(1..=59).contains(&step) {
            return Err("minute step must be between 1 and 59".to_owned());
        }
        return Ok(MinuteMatcher {
            mode: MinuteMatchMode::Every(step),
        });
    }

    let value = trimmed
        .parse::<u32>()
        .map_err(|_| "invalid minute value in cron expression".to_owned())?;
    if value > 59 {
        return Err("minute value must be between 0 and 59".to_owned());
    }
    Ok(MinuteMatcher {
        mode: MinuteMatchMode::Exact(value),
    })
}

#[cfg(test)]
mod tests {
    use crate::domain::models::CronSchedule;

    use super::compute_next_run_ms;

    #[test]
    fn cron_every_schedule_computes_next_run() {
        let schedule = CronSchedule {
            kind: "every".to_owned(),
            at: None,
            every_ms: Some(1_000),
            anchor_ms: None,
            expr: None,
            tz: None,
            stagger_ms: None,
        };
        let next = compute_next_run_ms(&schedule, 10).expect("next run should compute");
        assert_eq!(next, Some(1_010));
    }

    #[test]
    fn cron_supports_simple_star_expression() {
        let schedule = CronSchedule {
            kind: "cron".to_owned(),
            at: None,
            every_ms: None,
            anchor_ms: None,
            expr: Some("* * * * *".to_owned()),
            tz: Some("UTC".to_owned()),
            stagger_ms: None,
        };
        let now = 1_700_000_000_000_u64;
        let next = compute_next_run_ms(&schedule, now).expect("cron next run should compute");
        assert!(next.expect("next run should exist") > now);
    }
}
