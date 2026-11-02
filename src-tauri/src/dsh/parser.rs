//! Strict parsing of the public dsh-codex-subscription 3.2.3 projections.
//! No local Codex file, pipe, OAuth credential or alternate provider is consulted.
use super::{Account, Session};
use crate::codex::{QuotaSnapshot, RawCodexUsage};
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn text(value: &Value, max: usize) -> Result<String, String> {
    let text = value.as_str().ok_or("Malformed DSH text field")?;
    if text.is_empty() || text.len() > max || text.chars().any(char::is_control) {
        return Err("Malformed DSH text field".into());
    }
    Ok(text.into())
}

pub(super) fn rpc_response(value: Value, rpc_id: &str) -> Result<Value, String> {
    if value.get("type").and_then(Value::as_str) != Some("server-response")
        || value.get("rpcId").and_then(Value::as_str) != Some(rpc_id)
    {
        return Err("DSH RPC response identity mismatch".into());
    }
    // Remote error messages may contain URLs or authentication material; never
    // forward them to the frontend, logs, or persisted settings.
    let result = value.get("result").ok_or("Malformed DSH RPC response")?;
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("DSH RPC request was rejected".into());
    }
    result
        .get("value")
        .cloned()
        .ok_or_else(|| "Missing DSH RPC value".into())
}

pub(super) fn accounts(value: &Value) -> Result<Vec<Account>, String> {
    if value.get("authenticated").and_then(Value::as_bool) != Some(true)
        || value.get("provider").and_then(Value::as_str) != Some("openai-codex")
    {
        return Err("DSH Codex subscription is not authenticated".into());
    }
    let rows = value
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or("Missing DSH account identities")?;
    if rows.is_empty() || rows.len() > 256 {
        return Err("Invalid DSH account list".into());
    }
    let mut ids = HashSet::new();
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id = text(&row["id"], 256)?;
        if !ids.insert(id.clone()) {
            return Err("Duplicate DSH account identity".into());
        }
        result.push(Account {
            id,
            label: text(&row["label"], 256)?,
            active: row["active"]
                .as_bool()
                .ok_or("Missing DSH active account state")?,
        });
    }
    if result.iter().filter(|account| account.active).count() != 1 {
        return Err("DSH must expose exactly one active Codex account".into());
    }
    Ok(result)
}

pub(super) fn active_account(accounts: &[Account], requested: &str) -> Result<String, String> {
    if requested.is_empty() {
        return Err("Select an explicit DSH active account".into());
    }
    match accounts.iter().find(|account| account.active) {
        Some(account) if account.id == requested => Ok(account.id.clone()),
        _ => Err("DSH account changed or is not active; select the current active account".into()),
    }
}

pub(super) fn consistent_identity(
    before: &[Account],
    after: &[Account],
    requested: &str,
) -> Result<(), String> {
    active_account(before, requested)?;
    active_account(after, requested)?;
    let before_ids: HashSet<_> = before.iter().map(|a| a.id.as_str()).collect();
    let after_ids: HashSet<_> = after.iter().map(|a| a.id.as_str()).collect();
    if before_ids != after_ids {
        return Err("DSH account identities changed during the request".into());
    }
    Ok(())
}

pub(super) fn consistent_usage_identity(
    before: &[Account],
    reading: &Value,
    after: &[Account],
    requested: &str,
) -> Result<(), String> {
    consistent_identity(before, after, requested)?;
    let rows = reading["accounts"]
        .as_array()
        .ok_or("Missing DSH account usage identities")?;
    if rows.len() > 256 {
        return Err("Invalid DSH account usage identities".into());
    }
    let mut ids = HashSet::new();
    for row in rows {
        if !ids.insert(text(&row["id"], 256)?) {
            return Err("Duplicate DSH account usage identity".into());
        }
    }
    let expected: HashSet<_> = before.iter().map(|account| account.id.clone()).collect();
    if ids != expected {
        return Err("DSH usage account identities changed during the request".into());
    }
    Ok(())
}

pub(super) fn usage(value: &Value, account_id: &str, now_ms: i64) -> Result<RawCodexUsage, String> {
    let rows = value
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or("Missing DSH account usage list")?;
    if rows.is_empty() || rows.len() > 256 {
        return Err("Invalid DSH account usage list".into());
    }
    let mut seen = HashSet::new();
    let mut selected = None;
    for row in rows {
        let id = text(&row["id"], 256)?;
        if !seen.insert(id.clone()) {
            return Err("Duplicate DSH account usage identity".into());
        }
        if id == account_id {
            selected = Some(row);
        }
    }
    let row = selected.ok_or("Selected DSH account has no usage response")?;
    if row.get("error").is_some() {
        return Err("Could not read the selected DSH account usage".into());
    }
    let reading = row
        .get("usage")
        .ok_or("Missing selected DSH account usage")?;
    let fetched_ms = reading["fetchedAt"]
        .as_i64()
        .ok_or("Invalid DSH usage fetchedAt")?;
    if fetched_ms < 1_577_836_800_000
        || fetched_ms > now_ms.saturating_add(5_000)
        || fetched_ms < now_ms.saturating_sub(300_000)
    {
        return Err("DSH usage timestamp is invalid or stale".into());
    }
    let fetched_at = fetched_ms / 1000;
    let limits = reading["rateLimits"]
        .as_array()
        .ok_or("Missing DSH Codex rate limits")?;
    let codex: Vec<_> = limits
        .iter()
        .filter(|limit| limit["id"] == "codex")
        .collect();
    if codex.len() != 1 {
        return Err("Missing or ambiguous DSH Codex rate limit".into());
    }
    let windows = codex[0]["windows"]
        .as_array()
        .ok_or("Missing DSH Codex rate-limit windows")?;
    if windows.is_empty() || windows.len() > 16 {
        return Err("Invalid DSH Codex rate-limit windows".into());
    }
    let mut five_hour = None;
    let mut weekly = None;
    for window in windows {
        let seconds = window["windowSeconds"]
            .as_u64()
            .ok_or("Invalid DSH quota duration")?;
        let target = match seconds {
            18_000 => &mut five_hour,
            604_800 => &mut weekly,
            _ => return Err("Unsupported DSH Codex quota window duration".into()),
        };
        if target.is_some() {
            return Err("Duplicate DSH Codex quota window duration".into());
        }
        let used = window["usedPercent"]
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            .ok_or("Invalid DSH quota percentage")?;
        if let Some(remaining) = window.get("remainingPercent") {
            let remaining = remaining
                .as_f64()
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .ok_or("Invalid DSH remaining quota percentage")?;
            if (used + remaining - 100.0).abs() > 0.000_001 {
                return Err("Inconsistent DSH quota percentages".into());
            }
        }
        let reset = window["resetsAt"]
            .as_i64()
            .filter(|v| *v > fetched_at && *v <= fetched_at.saturating_add(seconds as i64 * 2))
            .ok_or("Invalid DSH quota reset timestamp")?;
        *target = Some(QuotaSnapshot {
            used_percent: Some(used),
            resets_at: Some(reset),
            window_duration_mins: Some((seconds / 60) as u32),
        });
    }
    let mut credits_balance = None;
    let mut has_credits = false;
    let mut credits_display = "—".to_owned();
    if let Some(credits) = reading.get("credits") {
        let unlimited = credits["unlimited"]
            .as_bool()
            .ok_or("Invalid DSH credit projection")?;
        has_credits = true;
        if let Some(balance) = credits.get("balance") {
            let value = text(balance, 64)?;
            let number = value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or("Invalid DSH credit balance")?;
            if !value.bytes().all(|c| c.is_ascii_digit() || c == b'.') || number > 1e15 {
                return Err("Invalid DSH credit balance".into());
            }
            credits_display = value.clone();
            credits_balance = Some(value);
        }
        if unlimited {
            credits_display = "Unlimited".into();
        }
    }
    Ok(RawCodexUsage {
        account_id: Some(account_id.into()),
        five_hour: five_hour.unwrap_or_default(),
        weekly: weekly.unwrap_or_default(),
        credits_display,
        credits_balance,
        has_credits,
        fetched_at,
    })
}

pub(super) fn fresh_usage(
    value: &Value,
    account_id: &str,
    now_ms: i64,
    started_ms: i64,
    previous_ms: Option<i64>,
) -> Result<(RawCodexUsage, i64), String> {
    let raw = usage(value, account_id, now_ms)?;
    let fetched_ms = value["accounts"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == account_id))
        .and_then(|row| row["usage"]["fetchedAt"].as_i64())
        .ok_or("Invalid DSH usage timestamp")?;
    // force:true still joins earlier flights; never relabel one as a new sample.
    if fetched_ms < started_ms || previous_ms.is_some_and(|previous| fetched_ms <= previous) {
        return Err(
            "DSH returned an earlier or duplicate quota observation; wait for the next refresh"
                .into(),
        );
    }
    Ok((raw, fetched_ms))
}

fn session_rows(value: &Value) -> Result<&Vec<Value>, String> {
    let rows = value["items"]
        .as_array()
        .ok_or("Malformed DSH session list")?;
    if rows.len() > 10_000 {
        return Err("DSH session list exceeds the limit".into());
    }
    let mut ids = HashSet::new();
    for row in rows {
        let id = text(&row["sessionId"], 256)?;
        if !ids.insert(id) {
            return Err("Duplicate DSH session identity".into());
        }
        if !row["running"].is_boolean()
            || !row["agentAvailable"].is_boolean()
            || !row["blank"].is_boolean()
        {
            return Err("Missing DSH session availability state".into());
        }
    }
    Ok(rows)
}
fn ordinary_session(row: &Value) -> bool {
    row["agentAvailable"] == true
        && row["blank"] == false
        && row.get("origin").is_none_or(Value::is_null)
        && row.get("parentSessionId").is_none_or(Value::is_null)
        && row
            .get("formatStatus")
            .is_none_or(|value| value == "current")
}
fn session_projection(row: &Value) -> Result<Session, String> {
    let provider = row
        .pointer("/projections/values/modelSelection/lastUsed/provider")
        .filter(|value| !value.is_null())
        .map(|value| text(value, 128))
        .transpose()?
        .unwrap_or_else(|| "unknown".into());
    let title = row
        .pointer("/projections/values/title")
        .filter(|value| !value.is_null())
        .map(|value| text(value, 1024))
        .transpose()?
        .unwrap_or_else(|| "Untitled".into());
    Ok(Session {
        id: text(&row["sessionId"], 256)?,
        title,
        provider,
        busy: row["running"] == true,
    })
}
pub(super) fn sessions(value: &Value) -> Result<Vec<Session>, String> {
    session_rows(value)?
        .iter()
        .filter(|row| ordinary_session(row))
        .map(session_projection)
        .collect()
}
pub(super) fn pause_target(value: &Value, session_id: &str) -> Result<Session, String> {
    let row = session_rows(value)?
        .iter()
        .find(|row| row["sessionId"] == session_id)
        .ok_or("DSH target session no longer exists")?;
    if !ordinary_session(row) || row["running"] != true {
        return Err("DSH pause target must be an existing running ordinary session".into());
    }
    let session = session_projection(row)?;
    if session.provider != "openai-codex" {
        return Err("DSH session is not running the Codex subscription provider".into());
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const NOW: i64 = 1_800_000_000_000;
    fn reading() -> Value {
        json!({"accounts":[{"id":"active-a","usage":{"fetchedAt":NOW-1000,"rateLimits":[{"id":"codex","windows":[
        {"windowSeconds":604800,"usedPercent":71.5,"remainingPercent":28.5,"resetsAt":NOW/1000+600000},
        {"windowSeconds":18000,"usedPercent":22,"remainingPercent":78,"resetsAt":NOW/1000+10000}]}]}}]})
    }
    fn identity() -> Vec<Account> {
        accounts(&json!({"authenticated":true,"provider":"openai-codex","accounts":[{"id":"active-a","label":"A","active":true},{"id":"b","label":"B","active":false}]})).unwrap()
    }
    #[test]
    fn maps_by_duration_not_ui_position_and_preserves_source_time() {
        let result = usage(&reading(), "active-a", NOW).unwrap();
        assert_eq!(result.five_hour.used_percent, Some(22.0));
        assert_eq!(result.weekly.used_percent, Some(71.5));
        assert_eq!(result.fetched_at, (NOW - 1000) / 1000);
        assert_eq!(result.account_id.as_deref(), Some("active-a"));
    }
    #[test]
    fn rejects_wrong_account_duplicates_invalid_times_and_numbers() {
        assert!(usage(&reading(), "b", NOW).is_err());
        let mut value = reading();
        let duplicate = value["accounts"][0].clone();
        value["accounts"].as_array_mut().unwrap().push(duplicate);
        assert!(usage(&value, "active-a", NOW).is_err());
        for invalid in [json!(-1), json!(101), json!("NaN"), json!(null)] {
            let mut value = reading();
            value["accounts"][0]["usage"]["rateLimits"][0]["windows"][0]["usedPercent"] = invalid;
            assert!(usage(&value, "active-a", NOW).is_err());
        }
        for timestamp in [NOW + 5001, NOW - 300001, 0] {
            let mut value = reading();
            value["accounts"][0]["usage"]["fetchedAt"] = json!(timestamp);
            assert!(usage(&value, "active-a", NOW).is_err());
        }
        let mut value = reading();
        value["accounts"][0]["usage"]["rateLimits"][0]["windows"][0]["resetsAt"] = json!(0);
        assert!(usage(&value, "active-a", NOW).is_err());
        let mut value = reading();
        value["accounts"][0]["usage"]["rateLimits"][0]["windows"][0]["windowSeconds"] = json!(3600);
        assert!(usage(&value, "active-a", NOW).is_err());
    }
    #[test]
    fn account_switch_and_nonactive_selection_fail_closed() {
        let before = identity();
        let mut after = before.clone();
        after[0].active = false;
        after[1].active = true;
        assert!(consistent_identity(&before, &after, "active-a").is_err());
        assert!(consistent_identity(&before, &before, "b").is_err());
        assert!(consistent_identity(&before, &before, "").is_err());
        assert!(consistent_identity(&before, &before, "active-a").is_ok());
        let mut after = before.clone();
        after.pop();
        assert!(consistent_identity(&before, &after, "active-a").is_err());
    }
    #[test]
    fn all_three_responses_must_have_the_same_account_id_set() {
        let before = identity();
        assert!(consistent_usage_identity(&before, &reading(), &before, "active-a").is_err());
        let mut complete = reading();
        complete["accounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","error":"Unavailable"}));
        assert!(consistent_usage_identity(&before, &complete, &before, "active-a").is_ok());
        complete["accounts"][1]["id"] = json!("unexpected");
        assert!(consistent_usage_identity(&before, &complete, &before, "active-a").is_err());
    }
    #[test]
    fn rejects_earlier_flights_duplicate_and_backwards_observations() {
        assert!(fresh_usage(&reading(), "active-a", NOW, NOW, None).is_err());
        assert!(fresh_usage(&reading(), "active-a", NOW, NOW - 1000, Some(NOW - 1000)).is_err());
        assert!(fresh_usage(&reading(), "active-a", NOW, NOW - 1000, Some(NOW)).is_err());
        assert!(fresh_usage(&reading(), "active-a", NOW, NOW - 1000, Some(NOW - 1001)).is_ok());
    }
    #[test]
    fn pause_targets_require_existing_running_ordinary_last_used_route() {
        let original = json!({"items":[{"sessionId":"ordinary","agentAvailable":true,"running":true,"blank":false,"formatStatus":"current","projections":{"kind":"cached","asOfSeq":1,"values":{"title":"Work","modelSelection":{"lastUsed":{"provider":"openai-codex"},"next":{"provider":"other"}}}}}]});
        assert!(pause_target(&original, "ordinary").is_ok());
        assert!(pause_target(&original, "missing").is_err());
        for (field, value) in [
            ("running", json!(false)),
            ("agentAvailable", json!(false)),
            ("blank", json!(true)),
            ("origin", json!("subagent")),
            ("parentSessionId", json!("parent")),
            ("formatStatus", json!("migration-required")),
        ] {
            let mut listed = original.clone();
            listed["items"][0][field] = value;
            assert!(pause_target(&listed, "ordinary").is_err());
        }
        let mut listed = original.clone();
        listed["items"][0]["projections"]["values"]["modelSelection"]["lastUsed"] = json!(null);
        listed["items"][0]["projections"]["values"]["modelSelection"]["next"] =
            json!({"provider":"openai-codex"});
        assert!(pause_target(&listed, "ordinary").is_err());
        let mut listed = original.clone();
        listed["items"][0]["projections"]["values"]["modelSelection"]["lastUsed"]["provider"] =
            json!("other");
        assert!(pause_target(&listed, "ordinary").is_err());
    }
    #[test]
    fn validates_rpc_identity_and_redacts_server_errors() {
        let value =
            json!({"type":"server-response","rpcId":"a","result":{"ok":true,"value":{"x":1}}});
        assert_eq!(rpc_response(value.clone(), "a").unwrap(), json!({"x":1}));
        assert!(rpc_response(value, "b").is_err());
        let value = json!({"type":"server-response","rpcId":"a","result":{"ok":false,"error":{"message":"https://host?token=SECRET"}}});
        assert!(!rpc_response(value, "a").unwrap_err().contains("SECRET"));
    }
}
