//! Observability setup: structured logging.
//!
//! **Important**: This module never writes to stdout, which is reserved for
//! application output (e.g., MCP server communication). All logging goes to
//! files or stderr.

use anyhow::Result;
use serde_json::{Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::Event;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Context as LayerContext, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

const ENV_LOG_PATH: &str = "SCRAT_LOG_PATH";
const ENV_LOG_DIR: &str = "SCRAT_LOG_DIR";
const DEFAULT_LOG_DIR_UNIX: &str = "/var/log";
const LOG_FILE_SUFFIX: &str = ".jsonl";

/// Configuration for observability setup.
#[derive(Clone, Debug)]
pub struct ObservabilityConfig {
    /// The service name used in log entries and traces.
    pub service: String,
    /// Directory for JSONL log files. Falls back to platform defaults if unset.
    pub log_dir: Option<PathBuf>,
}

impl ObservabilityConfig {
    /// Create config from environment variables with optional overrides.
    pub fn from_env_with_overrides(log_dir: Option<PathBuf>) -> Self {
        Self {
            service: env!("CARGO_PKG_NAME").to_string(),
            log_dir,
        }
    }
}

#[derive(Clone, Debug)]
struct LogTarget {
    dir: PathBuf,
    file_name: String,
}

impl LogTarget {
    #[cfg(test)]
    fn path(&self) -> PathBuf {
        self.dir.join(&self.file_name)
    }
}

/// Guard that must be held for the lifetime of the application to ensure
/// proper cleanup of logging and tracing resources.
pub struct ObservabilityGuard {
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Initialize observability (logging).
///
/// Returns a guard that must be held for the application lifetime.
///
/// # Errors
///
/// Returns an error if no writable log directory can be found and stderr
/// fallback is not acceptable for your use case.
pub fn init_observability(
    cfg: &ObservabilityConfig,
    env_filter: EnvFilter,
) -> Result<ObservabilityGuard> {
    let (log_writer, log_guard) = match build_log_writer(&cfg.service, cfg.log_dir.as_deref()) {
        Ok(result) => result,
        Err(err) => {
            // IMPORTANT: Fall back to stderr, NOT stdout.
            // stdout is reserved for application output (e.g., MCP servers).
            eprintln!("Warning: {err}. Falling back to stderr logging.");
            let (writer, guard) = tracing_appender::non_blocking(std::io::stderr());
            (writer, guard)
        }
    };

    let log_layer = JsonLogLayer::new(log_writer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(log_layer)
        .init();

    tracing::debug!("observability initialized");

    Ok(ObservabilityGuard {
        _log_guard: log_guard,
    })
}

/// Build an `EnvFilter` based on CLI flags and environment.
///
/// Priority: quiet flag > verbose flag > RUST_LOG env > default_level
pub fn env_filter(quiet: bool, verbose: u8, default_level: &str) -> EnvFilter {
    if quiet {
        return EnvFilter::new("error");
    }

    if verbose > 0 {
        let level = match verbose {
            1 => "debug",
            _ => "trace",
        };
        return EnvFilter::new(level);
    }

    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level))
}

// ============================================================================
// JSON Log Layer
// ============================================================================

struct JsonLogLayer<W> {
    writer: W,
}

impl<W> JsonLogLayer<W> {
    const fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<S, W> tracing_subscriber::Layer<S> for JsonLogLayer<W>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'writer> tracing_subscriber::fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: LayerContext<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = JsonVisitor::default();
            attrs.record(&mut visitor);
            span.extensions_mut().insert(SpanFields {
                values: visitor.values,
            });
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: LayerContext<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = JsonVisitor::default();
            values.record(&mut visitor);
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<SpanFields>() {
                fields.values.extend(visitor.values);
            } else {
                extensions.insert(SpanFields {
                    values: visitor.values,
                });
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: LayerContext<'_, S>) {
        let mut map = Map::new();

        let timestamp = format_timestamp();
        map.insert("timestamp".to_string(), Value::String(timestamp));
        map.insert(
            "level".to_string(),
            Value::String(event.metadata().level().as_str().to_lowercase()),
        );
        map.insert(
            "target".to_string(),
            Value::String(event.metadata().target().to_string()),
        );

        // Include span context in log entries
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    map.extend(fields.values.clone());
                }
            }
        }

        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        map.extend(visitor.values);

        let mut writer = self.writer.make_writer();
        if serde_json::to_writer(&mut writer, &Value::Object(map)).is_ok() {
            let _ = writer.write_all(b"\n");
        }
    }
}

#[derive(Clone, Debug)]
struct SpanFields {
    values: Map<String, Value>,
}

#[derive(Default)]
struct JsonVisitor {
    values: Map<String, Value>,
}

impl tracing::field::Visit for JsonVisitor {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.values
            .insert(field.name().to_string(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.values
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.values
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if let Some(number) = serde_json::Number::from_f64(value) {
            self.values
                .insert(field.name().to_string(), Value::Number(number));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.values
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.values
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.values.insert(
            field.name().to_string(),
            Value::String(format!("{value:?}")),
        );
    }
}

/// Format timestamp as RFC 3339 using std::time (no external time crate needed).
fn format_timestamp() -> String {
    // Use std::time for basic timestamp - good enough for logging
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let nanos = now.subsec_nanos();

    // Convert to datetime components (UTC)
    let days_since_epoch = secs / 86400;
    let secs_of_day = secs % 86400;
    let hours = secs_of_day / 3600;
    let minutes = (secs_of_day % 3600) / 60;
    let seconds = secs_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days_since_epoch as i64);

    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z",
        millis = nanos / 1_000_000
    )
}

/// Convert days since Unix epoch to (year, month, day).
const fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant's date algorithms
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
    (y as i32, m, d)
}

// ============================================================================
// Log Target Resolution
// ============================================================================

fn build_log_writer(
    service: &str,
    config_log_dir: Option<&Path>,
) -> Result<(
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    let target = resolve_log_target(service, config_log_dir).map_err(|e| anyhow::anyhow!("{e}"))?;

    let appender = tracing_appender::rolling::daily(&target.dir, &target.file_name);
    let (writer, guard) = tracing_appender::non_blocking(appender);

    Ok((writer, guard))
}

fn resolve_log_target(service: &str, config_log_dir: Option<&Path>) -> Result<LogTarget, String> {
    let path_override = std::env::var_os(ENV_LOG_PATH).map(PathBuf::from);
    let dir_override = std::env::var_os(ENV_LOG_DIR).map(PathBuf::from);

    resolve_log_target_with(
        service,
        path_override,
        dir_override,
        config_log_dir.map(PathBuf::from),
    )
}

fn resolve_log_target_with(
    service: &str,
    path_override: Option<PathBuf>,
    dir_override: Option<PathBuf>,
    config_dir: Option<PathBuf>,
) -> Result<LogTarget, String> {
    if let Some(path) = path_override {
        return log_target_from_path(path);
    }

    if let Some(dir) = dir_override {
        return log_target_from_dir(dir, service);
    }

    if let Some(dir) = config_dir {
        return log_target_from_dir(dir, service);
    }

    let mut candidates = Vec::new();

    if cfg!(unix) {
        candidates.push(PathBuf::from(DEFAULT_LOG_DIR_UNIX));
    }

    // Use XDG-compliant data directory for log storage
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", service) {
        candidates.push(proj_dirs.data_local_dir().join("logs"));
    }

    if let Ok(dir) = std::env::current_dir() {
        candidates.push(dir);
    }

    let file_name = format!("{service}{LOG_FILE_SUFFIX}");

    for dir in candidates {
        if ensure_writable(&dir, &file_name).is_ok() {
            return Ok(LogTarget { dir, file_name });
        }
    }

    Err("No writable log directory found".to_string())
}

fn log_target_from_dir(dir: PathBuf, service: &str) -> Result<LogTarget, String> {
    let file_name = format!("{service}{LOG_FILE_SUFFIX}");
    ensure_writable(&dir, &file_name)?;
    Ok(LogTarget { dir, file_name })
}

fn log_target_from_path(path: PathBuf) -> Result<LogTarget, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{ENV_LOG_PATH} must include a file name"))
        .and_then(|name| {
            name.to_str()
                .map(|value| value.to_string())
                .ok_or_else(|| format!("{ENV_LOG_PATH} must be valid UTF-8"))
        })?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_writable(dir, &file_name)?;

    Ok(LogTarget {
        dir: dir.to_path_buf(),
        file_name,
    })
}

fn ensure_writable(dir: &Path, file_name: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create log directory {}: {e}", dir.display()))?;

    let path = dir.join(file_name);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open log file {}: {e}", path.display()))?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_filter_quiet_overrides() {
        let filter = env_filter(true, 0, "info");
        assert_eq!(filter.to_string(), "error");
    }

    #[test]
    fn env_filter_verbose_maps_to_debug_and_trace() {
        let debug_filter = env_filter(false, 1, "info");
        assert_eq!(debug_filter.to_string(), "debug");

        let trace_filter = env_filter(false, 2, "info");
        assert_eq!(trace_filter.to_string(), "trace");
    }

    #[test]
    fn log_target_from_path_uses_parent_dir() {
        let temp_dir = std::env::temp_dir().join("scrat-log-path");
        let file_path = temp_dir.join("custom.jsonl");

        let target = log_target_from_path(file_path).expect("log target from path");
        assert_eq!(target.dir, temp_dir);
        assert_eq!(target.file_name, "custom.jsonl");
    }

    #[test]
    fn log_target_from_dir_appends_file_name() {
        let temp_dir = std::env::temp_dir().join("scrat-log-dir");
        let target = log_target_from_dir(temp_dir.clone(), "demo").expect("log target from dir");
        assert_eq!(target.dir, temp_dir);
        assert_eq!(target.file_name, format!("demo{LOG_FILE_SUFFIX}"));
    }

    #[test]
    fn resolve_log_target_with_prefers_path_override() {
        let temp_dir = std::env::temp_dir().join("scrat-log-override");
        let file_path = temp_dir.join("override.jsonl");

        let target = resolve_log_target_with("demo", Some(file_path.clone()), None, None)
            .expect("override log target");

        assert_eq!(target.path(), file_path);
    }

    #[test]
    fn resolve_log_target_with_falls_back_to_dir_override() {
        let temp_dir = std::env::temp_dir().join("scrat-log-dir-override");
        let target = resolve_log_target_with("demo", None, Some(temp_dir.clone()), None)
            .expect("dir override log target");

        assert_eq!(target.dir, temp_dir);
        assert_eq!(target.file_name, format!("demo{LOG_FILE_SUFFIX}"));
    }

    #[test]
    fn resolve_log_target_with_uses_config_dir() {
        let temp_dir = std::env::temp_dir().join("scrat-log-config-dir");
        let target = resolve_log_target_with("demo", None, None, Some(temp_dir.clone()))
            .expect("config dir log target");

        assert_eq!(target.dir, temp_dir);
        assert_eq!(target.file_name, format!("demo{LOG_FILE_SUFFIX}"));
    }

    #[test]
    fn format_timestamp_produces_valid_rfc3339() {
        let ts = format_timestamp();
        // Basic format check: YYYY-MM-DDTHH:MM:SS.mmmZ
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
        assert_eq!(ts.len(), 24, "timestamp should be 24 chars: {ts}");
        assert_eq!(&ts[4..5], "-", "year-month separator");
        assert_eq!(&ts[7..8], "-", "month-day separator");
        assert_eq!(&ts[10..11], "T", "date-time separator");
    }

    #[test]
    fn days_to_ymd_known_dates() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // 2000-01-01 = day 10957
        assert_eq!(days_to_ymd(10957), (2000, 1, 1));
        // 2024-02-29 (leap year) = day 19782
        assert_eq!(days_to_ymd(19782), (2024, 2, 29));
    }
}
