//! Runtime telemetry (`{proxy}/api/colab/resources`), normalised.
//!
//! Colab reports `memory {totalBytes, freeBytes}`, `disks [{label,
//! totalBytes, freeBytes}]` and `gpus [{name, totalBytes, usedBytes}]`
//! (ProtoJSON, so byte counts may be strings). The UI draws RAM / Disk / GPU
//! meters, so the engine turns that into `usage` / `limit` / `percent`.

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    pub usage: Option<f64>,
    pub limit: Option<f64>,
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    pub ram: Option<Meter>,
    pub disk: Option<Meter>,
    pub gpu: Option<Meter>,
}

fn bytes(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn meter(usage: Option<f64>, limit: Option<f64>, name: Option<String>) -> Option<Meter> {
    if usage.is_none() && limit.is_none() {
        return None;
    }
    let percent = match (usage, limit) {
        (Some(usage), Some(limit)) if limit > 0.0 => Some((usage / limit * 100.0).clamp(0.0, 100.0)),
        _ => None,
    };
    Some(Meter {
        usage,
        limit,
        percent,
        name,
    })
}

/// Used/total from an entry carrying `usedBytes`, or `totalBytes - freeBytes`.
fn used_and_total(entry: &Value) -> (Option<f64>, Option<f64>) {
    let total = bytes(entry.get("totalBytes"));
    let used = bytes(entry.get("usedBytes")).or_else(|| {
        let free = bytes(entry.get("freeBytes"))?;
        Some((total? - free).max(0.0))
    });
    (used, total)
}

pub fn normalize(raw: &Value) -> Resources {
    let ram = raw.get("memory").and_then(|memory| {
        let (used, total) = used_and_total(memory);
        meter(used, total, None)
    });

    let disks = raw.get("disks").and_then(Value::as_array);
    let disk = disks
        .and_then(|disks| {
            disks
                .iter()
                .find(|disk| disk.get("label").and_then(Value::as_str) == Some("/"))
                .or_else(|| disks.first())
        })
        .and_then(|disk| {
            let (used, total) = used_and_total(disk);
            meter(used, total, None)
        });

    let gpu = raw
        .get("gpus")
        .and_then(Value::as_array)
        .and_then(|gpus| gpus.first())
        .and_then(|gpu| {
            let (used, total) = used_and_total(gpu);
            let name = gpu.get("name").and_then(Value::as_str).map(str::to_owned);
            meter(used, total, name)
        });

    Resources { ram, disk, gpu }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalises_colab_telemetry() {
        let resources = normalize(&json!({
            "memory": {"totalBytes": 1000, "freeBytes": 400},
            "disks": [
                {"label": "/content/drive", "totalBytes": 9, "freeBytes": 9},
                {"label": "/", "totalBytes": "100", "freeBytes": "50"}
            ],
            "gpus": [{"name": "Tesla T4", "totalBytes": 1000, "usedBytes": 200}],
        }));
        let ram = resources.ram.unwrap();
        assert_eq!((ram.usage, ram.limit, ram.percent), (Some(600.0), Some(1000.0), Some(60.0)));
        let disk = resources.disk.unwrap();
        assert_eq!(disk.percent, Some(50.0));
        let gpu = resources.gpu.unwrap();
        assert_eq!(gpu.name.as_deref(), Some("Tesla T4"));
        assert_eq!(gpu.percent, Some(20.0));
    }

    #[test]
    fn cpu_runtime_has_no_gpu_meter() {
        let resources = normalize(&json!({"memory": {"totalBytes": 10, "freeBytes": 5}, "gpus": []}));
        assert!(resources.gpu.is_none());
        assert!(resources.disk.is_none());
        assert_eq!(normalize(&json!({})), Resources::default());
    }
}
