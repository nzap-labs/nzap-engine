//! Compute-unit quota and consumption — the VS Code extension's status bar
//! and low-balance notifier, as data.
//!
//! Port of colab-studio's `quota.py`, itself a port of
//! `colab-vscode/src/colab/consumption/status-bar.ts` and `notifier.ts` on
//! top of `ConsumptionUserInfoSchema` (`colab-vscode/src/colab/client/v1/api.ts`):
//!
//! * `consumptionRateHourly` — CCU burn rate across every assigned VM
//! * `paidComputeUnitsBalance` — paid CCUs left
//! * `freeCcuQuotaInfo.remainingTokens` — free allowance in **milli**-CCUs
//!   (ProtoJSON Int64, so it may arrive as a string)
//!
//! Two routes carry these numbers: `v1/user-info` (refuses the CLI's OAuth
//! client with 403 for ordinary accounts) and the web app's
//! `/tun/m/ccu-info`. Both are normalised into one [`Quota`].

use serde::Serialize;
use serde_json::{Map, Value};

use super::client::ColabClient;
use crate::error::{Error, Result};

/// notifier.ts: `WARN_WHEN_LESS_THAN_MINUTES`
pub const WARN_WHEN_LESS_THAN_MINUTES: i64 = 30;
/// notifier.ts: `DEFAULT_SNOOZE_MINUTES`
pub const DEFAULT_SNOOZE_MINUTES: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tier {
    None,
    Pro,
    ProPlus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Low,
    Depleted,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    /// `user-info` or `ccu-info`.
    pub source: String,
    /// Unix seconds.
    pub fetched_at: f64,
    pub tier: Tier,
    pub paid_compute_units: f64,
    pub consumption_rate_hourly: f64,
    pub assignments_count: i64,
    pub free_ccu_remaining: Option<f64>,
    pub free_minutes_remaining: Option<i64>,
    pub paid_minutes_remaining: Option<i64>,
    /// Paid + free minutes at the current burn rate (None while nothing burns).
    pub minutes_remaining: Option<i64>,
    pub next_free_refill_at: Option<f64>,
    pub severity: Severity,
    pub signup_action: String,
    pub eligible_accelerators: Vec<String>,
    pub ineligible_accelerators: Vec<String>,
    /// `X.XX/hr`, the status-bar text.
    pub status_text: String,
    /// The extension's tooltip text, verbatim.
    pub tooltip: String,
    pub warn_below_minutes: i64,
    pub snooze_minutes: i64,
    pub errors: Vec<String>,
}

/// ProtoJSON numbers may arrive as strings (Int64); accept both.
fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn tier(raw: Option<&Value>) -> Tier {
    match raw.and_then(Value::as_str).unwrap_or_default().to_ascii_uppercase().as_str() {
        "SUBSCRIPTION_TIER_PRO" | "PRO" => Tier::Pro,
        "SUBSCRIPTION_TIER_PRO_PLUS" | "PRO_PLUS" => Tier::ProPlus,
        _ => Tier::None,
    }
}

/// Flatten either shape into upper-case model names: `user-info` sends
/// `[{"variant": "VARIANT_GPU", "models": ["T4"]}]`, `ccu-info` plain
/// `["T4", "L4"]`.
fn accelerators(items: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for item in items.as_array().into_iter().flatten() {
        match item {
            Value::String(name) => names.push(name.to_ascii_uppercase()),
            Value::Object(object) => {
                for model in object.get("models").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(model) = model.as_str() {
                        names.push(model.to_ascii_uppercase());
                    }
                }
            }
            _ => {}
        }
    }
    names
}

/// Merge every `<prefix>*` list (`eligibleGpus` + `eligibleTpus` …),
/// keeping first-seen order without duplicates.
fn collect(raw: &Map<String, Value>, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (key, value) in raw {
        if key.starts_with(prefix) && value.is_array() {
            for name in accelerators(value) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// status-bar.ts `approximateFreeMinutesRemaining`: 10-minute buckets.
pub fn approximate_free_minutes(remaining_tokens: f64, rate_hourly: f64) -> i64 {
    if remaining_tokens <= 0.0 || rate_hourly <= 0.0 {
        return 0;
    }
    let minutes = (remaining_tokens / 1000.0 / rate_hourly) * 60.0;
    ((minutes / 10.0).floor() as i64) * 10
}

/// The extension's `XhYm` rendering.
pub fn format_minutes(minutes: i64) -> String {
    format!("{}h{}m", minutes / 60, minutes % 60)
}

/// status-bar.ts `updateStatusBarItem` tooltip, verbatim.
pub fn tooltip(tier: Tier, paid: f64, rate: f64, assignments: i64, free_tokens: f64) -> String {
    let mut text = match tier {
        Tier::Pro => "You are subscribed to Colab Pro.",
        Tier::ProPlus => "You are subscribed to Colab Pro+.",
        Tier::None => "You are not subscribed.",
    }
    .to_owned();
    if paid <= 0.0 {
        text.push_str(
            "\n\nYou currently have zero compute units available. \
             Resources offered free of charge are not guaranteed.",
        );
        if free_tokens > 0.0 && rate > 0.0 {
            let minutes = approximate_free_minutes(free_tokens, rate);
            text.push_str(&format!(
                "\n\nAt your current usage level, your server(s) may last up to {}.",
                format_minutes(minutes)
            ));
        }
    } else {
        text.push_str(&format!(
            "\n\nAvailable: {paid:.2} compute units\
             \nUsage rate: approximately {rate:.2} per hour\
             \nYou have {assignments} active session(s)."
        ));
    }
    text
}

/// notifier.ts `getTierRelevantAction`.
pub fn signup_action(tier: Tier, has_paid_balance: bool) -> &'static str {
    match tier {
        Tier::Pro => "Upgrade to Pro+",
        Tier::ProPlus => "Purchase More CCUs",
        Tier::None if has_paid_balance => "Purchase More CCUs",
        Tier::None => "Sign Up for Colab",
    }
}

/// Normalise a `user-info` / `ccu-info` payload and derive the numbers the
/// extension shows (burn rate, time left, severity, tooltip).
pub fn summarize(raw: &Map<String, Value>, source: &str, now: f64) -> Quota {
    let tier = tier(raw.get("subscriptionTier"));
    let paid = number(raw.get("paidComputeUnitsBalance"))
        .or_else(|| number(raw.get("currentBalance")))
        .unwrap_or(0.0);
    let rate = number(raw.get("consumptionRateHourly")).unwrap_or(0.0);
    let assignments = number(raw.get("assignmentsCount")).unwrap_or(0.0) as i64;

    let free_info = raw.get("freeCcuQuotaInfo");
    let free_tokens = number(free_info.and_then(|info| info.get("remainingTokens")));
    let next_refill = number(free_info.and_then(|info| info.get("nextRefillTimestampSec")));

    let burning = rate > 0.0;
    let free_minutes = approximate_free_minutes(free_tokens.unwrap_or(0.0), rate);
    let paid_minutes = burning.then(|| ((paid / rate) * 60.0) as i64);
    let minutes_left = burning.then(|| paid_minutes.unwrap_or(0) + free_minutes);

    // notifier.ts: nothing to warn about while nothing is burning CCUs.
    let severity = match minutes_left {
        Some(minutes) if minutes <= 0 => Severity::Depleted,
        Some(minutes) if minutes <= WARN_WHEN_LESS_THAN_MINUTES => Severity::Low,
        _ => Severity::Ok,
    };

    Quota {
        source: source.to_owned(),
        fetched_at: now,
        tier,
        paid_compute_units: paid,
        consumption_rate_hourly: rate,
        assignments_count: assignments,
        free_ccu_remaining: free_tokens.map(|tokens| tokens / 1000.0),
        free_minutes_remaining: burning.then_some(free_minutes),
        paid_minutes_remaining: paid_minutes,
        minutes_remaining: minutes_left,
        next_free_refill_at: next_refill,
        severity,
        signup_action: signup_action(tier, paid > 0.0).to_owned(),
        eligible_accelerators: collect(raw, "eligible"),
        ineligible_accelerators: collect(raw, "ineligible"),
        status_text: format!("{rate:.2}/hr"),
        tooltip: tooltip(tier, paid, rate, assignments, free_tokens.unwrap_or(0.0)),
        warn_below_minutes: WARN_WHEN_LESS_THAN_MINUTES,
        snooze_minutes: DEFAULT_SNOOZE_MINUTES,
        errors: Vec::new(),
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default()
}

/// Try each Google route in turn; the first that answers wins.
pub async fn fetch(client: &ColabClient) -> Result<Quota> {
    let mut errors = Vec::new();
    for source in ["user-info", "ccu-info"] {
        let outcome = if source == "user-info" {
            client.get_user_info(true).await
        } else {
            client.get_ccu_info().await
        };
        match outcome {
            Ok(Value::Object(raw)) if !raw.is_empty() => {
                let mut quota = summarize(&raw, source, unix_now());
                quota.errors = errors;
                return Ok(quota);
            }
            Ok(_) => errors.push(format!("{source}: empty response")),
            // No point trying the next route with credentials Google refused.
            Err(error) if matches!(error, Error::AuthExpired(_) | Error::NotConnected) => {
                return Err(error)
            }
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }
    Err(Error::Colab {
        status: None,
        message: format!("Colab quota is unavailable ({})", errors.join("; ")),
        body: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn free_minutes_come_in_ten_minute_buckets() {
        // 5000 milli-CCU at 2 CCU/h = 2.5 h = 150 min.
        assert_eq!(approximate_free_minutes(5000.0, 2.0), 150);
        // 4000 milli-CCU at 3 CCU/h = 80 min.
        assert_eq!(approximate_free_minutes(4000.0, 3.0), 80);
        assert_eq!(approximate_free_minutes(0.0, 2.0), 0);
        assert_eq!(approximate_free_minutes(100.0, 0.0), 0);
        assert_eq!(format_minutes(150), "2h30m");
        assert_eq!(format_minutes(5), "0h5m");
    }

    #[test]
    fn ccu_info_payload_while_burning_free_tokens() {
        let quota = summarize(
            &object(json!({
                "currentBalance": 0,
                "consumptionRateHourly": 1.96,
                "eligibleGpus": ["T4"],
                "ineligibleGpus": ["A100", "L4"],
                "eligibleTpus": ["V5E1"],
                "freeCcuQuotaInfo": {"remainingTokens": "1000", "nextRefillTimestampSec": "1700000000"},
            })),
            "ccu-info",
            1.0,
        );
        assert_eq!(quota.tier, Tier::None);
        assert_eq!(quota.paid_compute_units, 0.0);
        assert_eq!(quota.free_ccu_remaining, Some(1.0));
        // 1 CCU at 1.96/h ≈ 30.6 min → 30.
        assert_eq!(quota.free_minutes_remaining, Some(30));
        assert_eq!(quota.minutes_remaining, Some(30));
        assert_eq!(quota.severity, Severity::Low);
        assert_eq!(quota.status_text, "1.96/hr");
        assert_eq!(quota.signup_action, "Sign Up for Colab");
        assert_eq!(quota.eligible_accelerators, vec!["T4", "V5E1"]);
        assert_eq!(quota.ineligible_accelerators, vec!["A100", "L4"]);
        assert_eq!(quota.next_free_refill_at, Some(1_700_000_000.0));
        assert!(quota.tooltip.contains("may last up to 0h30m"));
    }

    #[test]
    fn user_info_payload_for_a_pro_account() {
        let quota = summarize(
            &object(json!({
                "subscriptionTier": "SUBSCRIPTION_TIER_PRO",
                "paidComputeUnitsBalance": 100,
                "consumptionRateHourly": 2,
                "assignmentsCount": 1,
                "eligibleAccelerators": [{"variant": "VARIANT_GPU", "models": ["t4", "A100"]}],
            })),
            "user-info",
            1.0,
        );
        assert_eq!(quota.tier, Tier::Pro);
        assert_eq!(quota.paid_minutes_remaining, Some(3000));
        assert_eq!(quota.severity, Severity::Ok);
        assert_eq!(quota.signup_action, "Upgrade to Pro+");
        assert_eq!(quota.eligible_accelerators, vec!["T4", "A100"]);
        assert!(quota.tooltip.starts_with("You are subscribed to Colab Pro."));
        assert!(quota.tooltip.contains("Available: 100.00 compute units"));
        assert!(quota.tooltip.contains("You have 1 active session(s)."));
    }

    #[test]
    fn nothing_burning_means_no_estimate_and_no_warning() {
        let quota = summarize(&object(json!({"paidComputeUnitsBalance": 0})), "ccu-info", 1.0);
        assert_eq!(quota.minutes_remaining, None);
        assert_eq!(quota.free_minutes_remaining, None);
        assert_eq!(quota.severity, Severity::Ok);
    }

    #[test]
    fn depleted_when_nothing_is_left() {
        let quota = summarize(
            &object(json!({"paidComputeUnitsBalance": 0, "consumptionRateHourly": 5})),
            "ccu-info",
            1.0,
        );
        assert_eq!(quota.minutes_remaining, Some(0));
        assert_eq!(quota.severity, Severity::Depleted);
    }

    #[test]
    fn serializes_camel_case() {
        let quota = summarize(&object(json!({"subscriptionTier": "PRO_PLUS"})), "ccu-info", 1.0);
        let json = serde_json::to_value(&quota).unwrap_or_default();
        assert_eq!(json["tier"], "PRO_PLUS");
        assert_eq!(json["severity"], "ok");
        assert!(json.get("consumptionRateHourly").is_some());
        assert_eq!(json["signupAction"], "Purchase More CCUs");
    }
}
