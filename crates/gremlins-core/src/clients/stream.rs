use std::io::Write;
use std::time::SystemTime;

pub(crate) fn ts_internal() -> String {
    // Manual UTC formatting to avoid chrono dependency
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // Compute date/time components from UNIX epoch seconds
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since epoch to year/month/day using civil calendar
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn trunc(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        format!("{}...", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

pub fn emit_init(prefix: &str, model: &str, cwd: &str) {
    eprintln!(
        "{} {}init model={} cwd={}",
        ts_internal(),
        prefix,
        model,
        cwd
    );
}

pub fn emit_text(prefix: &str, text: &str) {
    eprintln!("{} {}text: {}", ts_internal(), prefix, trunc(text, 200));
}

pub fn emit_think(prefix: &str, thinking: &str) {
    eprintln!(
        "{} {}think: {}",
        ts_internal(),
        prefix,
        trunc(thinking, 200)
    );
}

pub fn emit_tool(prefix: &str, name: &str, arg: &str) {
    eprintln!(
        "{} {}tool: {} {}",
        ts_internal(),
        prefix,
        name,
        trunc(arg, 200)
    );
}

pub fn emit_result(prefix: &str, content: &str, is_error: bool) {
    let err = if is_error { " ERROR" } else { "" };
    eprintln!(
        "{} {}result{}: {}",
        ts_internal(),
        prefix,
        err,
        trunc(content, 200)
    );
}

pub fn flush() {
    let _ = std::io::stderr().flush();
}
