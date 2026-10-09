//! A cell's events as the agent reads them: output text, results, errors
//! (tracebacks without terminal colours) and small images.

use serde_json::Value;

/// The kernel MIME type NZAP apps report progress and results with.
pub const APP_EVENT_MIME: &str = "application/vnd.nzap.app+json";

/// What the agent sees of the text: the end, where the result and any
/// error are.
const SHOWN_CHARS: usize = 30_000;
/// Kept while running, so a chatty cell cannot exhaust memory.
const KEPT_CHARS: usize = 256 * 1024;
const MAX_IMAGES: usize = 4;
const MAX_IMAGE_BASE64: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub struct Transcript {
    text: String,
    dropped: usize,
    /// `(mime, base64)`.
    pub images: Vec<(String, String)>,
    pub errors: Vec<String>,
    /// `ok` | `error` | `aborted`, from the final `execute_reply`.
    pub status: Option<String>,
}

/// Strip ANSI escape sequences (IPython colours its tracebacks).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn plain(data: &Value) -> Option<String> {
    match data.get("text/plain")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(lines) => Some(lines.iter().filter_map(Value::as_str).collect()),
        _ => None,
    }
}

impl Transcript {
    fn write(&mut self, text: &str) {
        self.text.push_str(text);
        if self.text.len() > KEPT_CHARS {
            let mut cut = self.text.len() - KEPT_CHARS;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.dropped += cut;
            self.text.drain(..cut);
        }
    }

    pub fn push(&mut self, event: &Value) {
        match event.get("type").and_then(Value::as_str).unwrap_or_default() {
            "stream" => {
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    self.write(&strip_ansi(text));
                }
            }
            "result" | "display" | "update_display" => {
                let data = event.get("data").cloned().unwrap_or(Value::Null);
                if data.get(APP_EVENT_MIME).is_some() {
                    return;
                }
                for mime in ["image/png", "image/jpeg"] {
                    if let Some(encoded) = data.get(mime).and_then(Value::as_str) {
                        let encoded: String =
                            encoded.chars().filter(|c| !c.is_whitespace()).collect();
                        if self.images.len() < MAX_IMAGES && encoded.len() <= MAX_IMAGE_BASE64 {
                            self.images.push((mime.to_owned(), encoded));
                            self.write(&format!("[image {}]\n", self.images.len()));
                        } else {
                            self.write("[image omitted]\n");
                        }
                        return;
                    }
                }
                if let Some(text) = plain(&data) {
                    self.write(&strip_ansi(&text));
                    self.write("\n");
                }
            }
            "error" => {
                let ename = event.get("ename").and_then(Value::as_str).unwrap_or("Error");
                let evalue = event.get("evalue").and_then(Value::as_str).unwrap_or_default();
                let traceback: Vec<String> = event
                    .get("traceback")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(strip_ansi)
                    .collect();
                if traceback.is_empty() {
                    self.write(&format!("{ename}: {evalue}\n"));
                } else {
                    self.write(&traceback.join("\n"));
                    self.write("\n");
                }
                self.errors.push(format!("{ename}: {evalue}").trim_end_matches(": ").to_owned());
            }
            "input_request" => {
                self.write("[input() is not available to agents; it received an empty line]\n");
            }
            "drive_auth_required" => {
                let uri = event.get("uri").and_then(Value::as_str).unwrap_or_default();
                self.write(&format!(
                    "[Google Drive / Cloud access needs the user's consent in a browser{}. \
                     The cell was interrupted. Ask the user to open the link (or mount Drive \
                     from the NZAP Engine app), then try again.]\n",
                    if uri.is_empty() { String::new() } else { format!(": {uri}") }
                ));
            }
            "colab_request" => {
                if let Some(message) = event.get("message").and_then(Value::as_str) {
                    self.write(&format!("[{message}]\n"));
                }
            }
            "execute_reply" => {
                self.status = event.get("status").and_then(Value::as_str).map(str::to_owned);
            }
            _ => {}
        }
    }

    /// The text the agent sees: the tail, with a marker for what was cut.
    pub fn render(&self) -> String {
        let total = self.dropped + self.text.len();
        if total <= SHOWN_CHARS {
            return self.text.clone();
        }
        let mut start = self.text.len().saturating_sub(SHOWN_CHARS);
        while !self.text.is_char_boundary(start) {
            start += 1;
        }
        format!(
            "[… {} earlier characters omitted …]\n{}",
            total - (self.text.len() - start),
            &self.text[start..]
        )
    }

    pub fn failed(&self) -> bool {
        !self.errors.is_empty() || self.status.as_deref().is_some_and(|status| status != "ok")
    }
}

/// An NZAP app event carried in a display message, if this is one.
pub fn app_event(event: &Value) -> Option<Value> {
    let kind = event.get("type").and_then(Value::as_str)?;
    if !matches!(kind, "display" | "update_display" | "result") {
        return None;
    }
    let payload = match event.get("data")?.get(APP_EVENT_MIME)? {
        Value::String(text) => serde_json::from_str(text).ok()?,
        other => other.clone(),
    };
    payload.get("event").and_then(Value::as_str).is_some().then_some(payload)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn collects_text_results_errors_and_images() {
        let mut transcript = Transcript::default();
        for event in [
            json!({"type": "stream", "name": "stdout", "text": "hello\n"}),
            json!({"type": "result", "data": {"text/plain": "42"}}),
            json!({"type": "display", "data": {"image/png": "aGVs\nbG8=", "text/plain": "<Figure>"}}),
            json!({"type": "error", "ename": "ValueError", "evalue": "boom",
                   "traceback": ["\u{1b}[0;31mValueError\u{1b}[0m: boom"]}),
            json!({"type": "execute_reply", "status": "error"}),
        ] {
            transcript.push(&event);
        }
        assert_eq!(transcript.render(), "hello\n42\n[image 1]\nValueError: boom\n");
        assert_eq!(transcript.images, vec![("image/png".to_owned(), "aGVsbG8=".to_owned())]);
        assert_eq!(transcript.errors, vec!["ValueError: boom"]);
        assert!(transcript.failed());
    }

    #[test]
    fn long_output_keeps_the_end() {
        let mut transcript = Transcript::default();
        for index in 0..20_000 {
            transcript.push(&json!({"type": "stream", "text": format!("line {index}\n")}));
        }
        let shown = transcript.render();
        assert!(shown.starts_with("[… "));
        assert!(shown.ends_with("line 19999\n"));
        assert!(shown.len() < SHOWN_CHARS + 100);
    }

    #[test]
    fn app_events_are_recognised_in_either_encoding() {
        let object =
            json!({"type": "display", "data": {APP_EVENT_MIME: {"event": "ready", "warm": true}}});
        let string = json!({"type": "display", "data": {APP_EVENT_MIME: "{\"event\": \"done\"}"}});
        assert_eq!(app_event(&object).unwrap()["warm"], true);
        assert_eq!(app_event(&string).unwrap()["event"], "done");
        assert!(app_event(&json!({"type": "stream", "text": "x"})).is_none());
        let mut transcript = Transcript::default();
        transcript.push(&object);
        assert_eq!(transcript.render(), "");
    }
}
