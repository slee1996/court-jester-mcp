use std::collections::HashSet;

use crate::types::{
    FailureDiagnostic, FailureDomain, FailureKind, HarnessEvent, HarnessEventRecord,
    InputClassification, ProcessTermination, ProcessTerminationKind,
};

pub const HARNESS_EVENT_SENTINEL: &str = "__COURT_JESTER_EVENT_JSON__";
pub const HARNESS_EVENT_PROTOCOL_VERSION: u32 = 1;
pub const HARNESS_EVENT_MAX_LINE_BYTES: usize = 262_144;
pub const HARNESS_EVENT_MAX_RECORDS: usize = 100_000;

#[derive(Debug, Clone)]
pub struct HarnessEventSummary {
    pub records: Vec<HarnessEventRecord>,
    pub findings: Vec<crate::types::VerificationFinding>,
    pub completed_units: usize,
    pub runner_started: bool,
    pub target_resolved: bool,
    pub target_ready: bool,
    pub harness_completed: bool,
    pub open_unit: Option<(String, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    Start,
    Bootstrap,
    Resolved,
    Ready,
    Completed,
    BootstrapFailed,
}

fn event_protocol_error(message: impl Into<String>) -> String {
    format!("harness protocol error: {}", message.into())
}

pub fn parse_harness_events(output: &str) -> Result<HarnessEventSummary, String> {
    let mut records = Vec::new();
    let mut seen_sequences = HashSet::new();
    let mut state = EventState::Start;
    let mut next_sequence = 0u64;
    let mut current_unit: Option<(String, usize)> = None;
    let mut findings = Vec::new();
    let mut completed_units = 0usize;

    for line in output.lines() {
        if !line
            .as_bytes()
            .starts_with(HARNESS_EVENT_SENTINEL.as_bytes())
        {
            continue;
        }
        if line.len() > HARNESS_EVENT_MAX_LINE_BYTES {
            return Err(event_protocol_error("event line exceeds 262144 bytes"));
        }
        if records.len() >= HARNESS_EVENT_MAX_RECORDS {
            return Err(event_protocol_error("event record limit exceeded"));
        }
        let payload = &line[HARNESS_EVENT_SENTINEL.len()..];
        if payload.starts_with(HARNESS_EVENT_SENTINEL) {
            return Err(event_protocol_error("duplicate event sentinel"));
        }
        let record = serde_json::from_str::<HarnessEventRecord>(payload)
            .map_err(|error| event_protocol_error(error.to_string()))?;
        if record.protocol_version != HARNESS_EVENT_PROTOCOL_VERSION {
            return Err(event_protocol_error(format!(
                "unsupported protocol version {}",
                record.protocol_version
            )));
        }
        if !seen_sequences.insert(record.sequence) {
            return Err(event_protocol_error("duplicate event sequence"));
        }
        if record.sequence != next_sequence {
            return Err(event_protocol_error(format!(
                "expected sequence {}, got {}",
                next_sequence, record.sequence
            )));
        }
        next_sequence = next_sequence.saturating_add(1);
        match &record.event {
            HarnessEvent::BootstrapStarted => {
                if state != EventState::Start {
                    return Err(event_protocol_error("bootstrap_started is not first"));
                }
                state = EventState::Bootstrap;
            }
            HarnessEvent::TargetResolved { module } => {
                if state != EventState::Bootstrap || module.is_empty() {
                    return Err(event_protocol_error("target_resolved before bootstrap"));
                }
                state = EventState::Resolved;
            }
            HarnessEvent::BootstrapFailed { .. } => {
                if !matches!(state, EventState::Bootstrap | EventState::Resolved) {
                    return Err(event_protocol_error("bootstrap_failed in invalid state"));
                }
                state = EventState::BootstrapFailed;
            }
            HarnessEvent::TargetReady => {
                if state != EventState::Resolved {
                    return Err(event_protocol_error("target_ready before target_resolved"));
                }
                state = EventState::Ready;
            }
            HarnessEvent::UnitStarted {
                surface_id,
                iteration,
                input_classification,
                ..
            } => {
                if state != EventState::Ready || current_unit.is_some() {
                    return Err(event_protocol_error(
                        "unit_started overlaps or precedes target",
                    ));
                }
                if *input_classification == InputClassification::Unknown {
                    // Unknown validity remains representable, but cannot silently
                    // become a target finding. The reducer handles its impact.
                }
                current_unit = Some((surface_id.clone(), *iteration));
            }
            HarnessEvent::Finding { finding } => {
                if current_unit.is_none() && state != EventState::Ready {
                    return Err(event_protocol_error(
                        "finding must be inside a unit or follow target_ready",
                    ));
                }
                findings.push(finding.clone());
            }
            HarnessEvent::UnitCompleted {
                surface_id,
                iteration,
                ..
            } => {
                if current_unit.as_ref() != Some(&(surface_id.clone(), *iteration)) {
                    return Err(event_protocol_error(
                        "unit_completed does not match unit_started",
                    ));
                }
                current_unit = None;
                completed_units = completed_units.saturating_add(1);
            }
            HarnessEvent::HarnessCompleted {
                completed_units: reported,
            } => {
                if state != EventState::Ready || current_unit.is_some() {
                    return Err(event_protocol_error("harness_completed before units close"));
                }
                if *reported != completed_units {
                    return Err(event_protocol_error("harness_completed count disagrees"));
                }
                state = EventState::Completed;
            }
        }
        records.push(record);
    }

    if state == EventState::Start {
        return Err(event_protocol_error("no bootstrap event"));
    }

    Ok(HarnessEventSummary {
        records,
        findings,
        completed_units,
        runner_started: state != EventState::Start,
        target_resolved: matches!(
            state,
            EventState::Resolved | EventState::Ready | EventState::Completed
        ),
        target_ready: matches!(state, EventState::Ready | EventState::Completed),
        harness_completed: state == EventState::Completed,
        open_unit: current_unit,
    })
}

fn termination(
    kind: ProcessTerminationKind,
    exit_code: Option<i32>,
    signal: Option<i32>,
) -> ProcessTermination {
    ProcessTermination {
        kind,
        exit_code,
        signal,
        signal_name: signal.map(signal_name),
    }
}

fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGTERM => "SIGTERM",
        libc::SIGKILL => "SIGKILL",
        libc::SIGXCPU => "SIGXCPU",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        other => return format!("SIG{other}"),
    }
    .to_string()
}

fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}
/// Describe the in-memory instrumentation overlay used for authoritative tests.
/// The original source is never rewritten; unsupported runner/import modes are
/// explicit and must be treated as inconclusive by verification.
pub fn build_instrumentation_overlay(
    language: &Language,
    runner: TestRunner,
    source_file: &str,
    surfaces: &[String],
) -> InstrumentationOverlay {
    let (mode, supported) = match language {
        Language::Python => (InstrumentationMode::PythonSitecustomize, true),
        Language::TypeScript => match runner {
            TestRunner::Bun => (InstrumentationMode::BunPreload, true),
            TestRunner::Node | TestRunner::Auto => (InstrumentationMode::NodeModuleRegister, true),
            TestRunner::RepoNative => (InstrumentationMode::Unsupported, false),
        },
    };
    InstrumentationOverlay {
        mode,
        source_file: source_file.to_string(),
        surfaces: surfaces.to_vec(),
        supported,
        reason: (!supported)
            .then(|| "repo-native runner does not expose a module transform hook".into()),
    }
}

use std::time::Instant;
use tokio::process::Command;

use crate::types::*;

#[cfg(target_os = "macos")]
fn get_rss_bytes(pid: u32) -> u64 {
    use std::mem;
    const PROC_PIDTASKINFO: i32 = 4;

    #[repr(C)]
    struct ProcTaskInfo {
        pti_virtual_size: u64,
        pti_resident_size: u64,
        pti_total_user: u64,
        pti_total_system: u64,
        pti_threads_user: u64,
        pti_threads_system: u64,
        pti_policy: i32,
        pti_faults: i32,
        pti_pageins: i32,
        pti_cow_faults: i32,
        pti_messages_sent: i32,
        pti_messages_received: i32,
        pti_syscalls_mach: i32,
        pti_syscalls_unix: i32,
        pti_csw: i32,
        pti_threadnum: i32,
        pti_numrunning: i32,
        pti_priority: i32,
    }

    unsafe {
        let mut info: ProcTaskInfo = mem::zeroed();
        let size = mem::size_of::<ProcTaskInfo>() as i32;
        unsafe extern "C" {
            fn proc_pidinfo(
                pid: i32,
                flavor: i32,
                arg: u64,
                buffer: *mut libc::c_void,
                buffersize: i32,
            ) -> i32;
        }
        let ret = proc_pidinfo(
            pid as i32,
            PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret > 0 {
            info.pti_resident_size
        } else {
            0
        }
    }
}

#[cfg(target_os = "macos")]
fn get_process_group_rss_bytes(pgid: u32) -> u64 {
    unsafe {
        unsafe extern "C" {
            fn proc_listpgrppids(
                pgrpid: libc::pid_t,
                buffer: *mut libc::c_void,
                buffersize: i32,
            ) -> i32;
        }

        let mut pids = vec![0i32; 256];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
        let filled = proc_listpgrppids(pgid as i32, pids.as_mut_ptr() as *mut libc::c_void, bytes);
        if filled <= 0 {
            return 0;
        }

        let pid_count = (filled as usize).min(pids.len());
        pids[..pid_count]
            .iter()
            .filter_map(|pid| (*pid > 0).then_some(*pid as u32))
            .map(get_rss_bytes)
            .sum()
    }
}

#[cfg(target_os = "linux")]
fn get_rss_bytes(pid: u32) -> u64 {
    let status = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn get_process_group_rss_bytes(pgid: u32) -> u64 {
    let mut total = 0;
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let pid = match entry.file_name().to_string_lossy().parse::<u32>() {
            Ok(pid) => pid,
            Err(_) => continue,
        };
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(_) => continue,
        };
        let (_, rest) = match stat.split_once(") ") {
            Some(parts) => parts,
            None => continue,
        };
        let mut fields = rest.split_whitespace();
        let _state = fields.next();
        let _ppid = fields.next();
        let row_pgid = match fields.next().and_then(|value| value.parse::<u32>().ok()) {
            Some(row_pgid) => row_pgid,
            None => continue,
        };
        if row_pgid == pgid {
            total += get_rss_bytes(pid);
        }
    }

    total
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_process_group_rss_bytes(_pgid: u32) -> u64 {
    0
}

/// Find a binary on the given PATH, returning its absolute path if found.
fn which_binary(path_env: &str, binary: &str) -> Option<String> {
    for dir in path_env.split(':') {
        let candidate = std::path::Path::new(dir).join(binary);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn is_valid_python_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn resolved_typescript_import(value: &str, source_dir: &std::path::Path) -> Option<String> {
    if !(value.starts_with("./") || value.starts_with("../"))
        || value.contains('?')
        || value.contains('#')
        || std::path::Path::new(value).extension().is_some()
    {
        return None;
    }
    let path = source_dir.join(value);
    for extension in ["ts", "tsx", "js", "jsx"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Some(format!("{value}.{extension}"));
        }
    }
    for extension in ["ts", "tsx", "js", "jsx"] {
        if path.join(format!("index.{extension}")).is_file() {
            return Some(format!("{value}/index.{extension}"));
        }
    }
    None
}

fn rewrite_typescript_relative_imports(
    code: &str,
    source_file: Option<&std::path::Path>,
) -> String {
    let Some(source_dir) = source_file.and_then(std::path::Path::parent) else {
        return code.to_string();
    };
    let mut rewritten = String::with_capacity(code.len());
    let mut cursor = 0;
    while cursor < code.len() {
        let Some(relative_quote) = code[cursor..]
            .find(['"', '\''])
            .map(|offset| cursor + offset)
        else {
            rewritten.push_str(&code[cursor..]);
            break;
        };
        rewritten.push_str(&code[cursor..relative_quote]);
        let quote = code.as_bytes()[relative_quote] as char;
        let Some(end_offset) = code[relative_quote + 1..].find(quote) else {
            rewritten.push_str(&code[relative_quote..]);
            break;
        };
        let end = relative_quote + 1 + end_offset;
        let value = &code[relative_quote + 1..end];
        let line_start = code[..relative_quote]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let line_prefix = code[line_start..relative_quote].trim_start();
        let import_context = line_prefix.contains(" from ")
            || line_prefix.starts_with("import ")
            || line_prefix.contains("import(")
            || line_prefix.contains("require(");
        if import_context {
            if let Some(resolved) = resolved_typescript_import(value, source_dir) {
                rewritten.push(quote);
                rewritten.push_str(&resolved);
                rewritten.push(quote);
                cursor = end + 1;
                continue;
            }
        }
        rewritten.push(quote);
        rewritten.push_str(value);
        rewritten.push(quote);
        cursor = end + 1;
    }
    rewritten
}

pub fn typescript_code_requires_bun_runtime(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("Bun.")
            || trimmed.contains("typeof Bun")
            || trimmed.contains("instanceof Bun")
            || trimmed.contains("from \"bun\"")
            || trimmed.contains("from 'bun'")
            || trimmed.contains("from \"bun:")
            || trimmed.contains("from 'bun:")
            || trimmed.contains("import \"bun\"")
            || trimmed.contains("import 'bun'")
            || trimmed.contains("require(\"bun\")")
            || trimmed.contains("require('bun')")
    })
}

fn dir_declares_bun_package_manager(dir: &std::path::Path) -> bool {
    let package_json = dir.join("package.json");
    std::fs::read_to_string(package_json)
        .map(|text| text.contains("\"packageManager\"") && text.contains("bun@"))
        .unwrap_or(false)
}

pub fn detect_repo_typescript_runner(
    project_dir: Option<&str>,
    source_file: Option<&str>,
) -> Option<String> {
    let mut starts = Vec::new();
    if let Some(dir) = project_dir {
        starts.push(std::path::PathBuf::from(dir));
    }
    if let Some(source_file) = source_file {
        if let Some(parent) = std::path::Path::new(source_file).parent() {
            starts.push(parent.to_path_buf());
        }
    }

    for start in starts {
        let mut dir = start.as_path();
        loop {
            if dir.join("bun.lock").exists()
                || dir.join("bun.lockb").exists()
                || dir_declares_bun_package_manager(dir)
            {
                return Some("bun".into());
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }
    }

    None
}

fn parse_quoted_path(input: &str) -> Option<String> {
    let quote = input.chars().find(|c| *c == '"' || *c == '\'')?;
    let start = input.find(quote)? + 1;
    let end = start + input[start..].find(quote)?;
    Some(input[start..end].to_string())
}

fn resolve_typescript_import_file(
    source_file: &str,
    import_path: &str,
) -> Option<std::path::PathBuf> {
    let source_dir = std::path::Path::new(source_file).parent()?;
    let base = source_dir.join(import_path);

    if base.exists() {
        return Some(base);
    }
    for ext in [".ts", ".tsx", "/index.ts", "/index.tsx"] {
        let candidate = std::path::PathBuf::from(format!("{}{}", base.display(), ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
fn extract_typescript_named_relative_imports(code: &str) -> Vec<(String, Vec<String>)> {
    let mut imports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("import ") || trimmed.starts_with("import type ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };
        let names = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry = entry.strip_prefix("type ").unwrap_or(entry);
                let export_name = entry
                    .split_once(" as ")
                    .map(|(name, _)| name)
                    .unwrap_or(entry)
                    .trim();
                (!export_name.is_empty()).then(|| export_name.to_string())
            })
            .collect::<Vec<_>>();
        if !names.is_empty() {
            imports.push((path, names));
        }
    }

    imports
}

#[derive(Clone, Debug)]
struct RelativeReexportSpecifier {
    source_name: String,
    exported_name: String,
    type_only: bool,
}

fn extract_typescript_named_relative_reexports(
    code: &str,
) -> Vec<(String, Vec<RelativeReexportSpecifier>)> {
    let mut reexports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("export ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };

        let clause = clause.trim();
        let statement_type_only = clause.starts_with("type ");
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };

        let specifiers = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry_type_only = statement_type_only || entry.starts_with("type ");
                let entry = entry.strip_prefix("type ").unwrap_or(entry).trim();
                let (source_name, exported_name) = entry
                    .split_once(" as ")
                    .map(|(source_name, exported_name)| (source_name.trim(), exported_name.trim()))
                    .unwrap_or((entry, entry));
                if source_name.is_empty() || exported_name.is_empty() {
                    return None;
                }
                Some(RelativeReexportSpecifier {
                    source_name: source_name.to_string(),
                    exported_name: exported_name.to_string(),
                    type_only: entry_type_only,
                })
            })
            .collect::<Vec<_>>();
        if !specifiers.is_empty() {
            reexports.push((path, specifiers));
        }
    }

    reexports
}

fn source_key(source_file: &str) -> String {
    std::fs::canonicalize(source_file)
        .unwrap_or_else(|_| std::path::PathBuf::from(source_file))
        .to_string_lossy()
        .to_string()
}

fn target_exports_name_only_as_type(
    code: &str,
    source_file: &str,
    name: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let visit_key = format!("{}::{name}", source_key(source_file));
    if !visited.insert(visit_key) {
        return false;
    }

    let type_alias_prefix = format!("export type {name}");
    let interface_prefix = format!("export interface {name}");

    if code.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(&type_alias_prefix) || trimmed.starts_with(&interface_prefix)
    }) {
        return true;
    }

    extract_typescript_named_relative_reexports(code)
        .into_iter()
        .any(|(import_path, specifiers)| {
            specifiers.into_iter().any(|specifier| {
                if specifier.exported_name != name {
                    return false;
                }
                if specifier.type_only {
                    return true;
                }
                let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                    Some(path) => path,
                    None => return false,
                };
                let imported_code = match std::fs::read_to_string(&resolved) {
                    Ok(code) => code,
                    Err(_) => return false,
                };
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    &specifier.source_name,
                    visited,
                )
            })
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports_inner(
    code: &str,
    source_file: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let source_key = source_key(source_file);
    if !visited.insert(source_key) {
        return false;
    }

    extract_typescript_named_relative_imports(code)
        .into_iter()
        .any(|(import_path, names)| {
            let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                Some(path) => path,
                None => return false,
            };
            let imported_code = match std::fs::read_to_string(&resolved) {
                Ok(code) => code,
                Err(_) => return false,
            };
            let mut export_visited = std::collections::HashSet::new();
            names.iter().any(|name| {
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    name,
                    &mut export_visited,
                )
            }) || has_typescript_type_only_relative_imports_inner(
                &imported_code,
                resolved.to_str().unwrap_or_default(),
                visited,
            )
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports(code: &str, source_file: Option<&str>) -> bool {
    let source_file = match source_file {
        Some(source_file) => source_file,
        None => return false,
    };

    has_typescript_type_only_relative_imports_inner(
        code,
        source_file,
        &mut std::collections::HashSet::new(),
    )
}

/// Remove only imports proven to contain type-only exports. Node 24's native
/// type stripping handles syntax, but it must not evaluate a type-only binding
/// from a runtime module. Unknown bindings are left untouched.
fn strip_typescript_type_only_imports(code: &str, source_file: Option<&str>) -> String {
    let Some(source_file) = source_file else {
        return code.to_string();
    };
    let mut out = String::with_capacity(code.len());
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import type ") {
            continue;
        }
        if trimmed.starts_with("import {") && trimmed.contains("} from") {
            let Some((left, right)) = line.split_once("} from") else {
                out.push_str(line);
                out.push('\n');
                continue;
            };
            let names = left.split_once('{').map(|(_, names)| names).unwrap_or("");
            let import_path = parse_quoted_path(right).unwrap_or_default();
            if !(import_path.starts_with("./") || import_path.starts_with("../")) {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let resolved = resolve_typescript_import_file(source_file, &import_path);
            let kept = names
                .split(',')
                .filter_map(|entry| {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        return None;
                    }
                    let source_name = entry
                        .split_once(" as ")
                        .map(|(name, _)| name.trim())
                        .unwrap_or(entry);
                    let is_type = resolved
                        .as_ref()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .map(|text| {
                            let mut visited = std::collections::HashSet::new();
                            target_exports_name_only_as_type(
                                &text,
                                resolved.as_ref().and_then(|p| p.to_str()).unwrap_or(""),
                                source_name,
                                &mut visited,
                            )
                        })
                        .unwrap_or(false);
                    (!is_type).then(|| entry.to_string())
                })
                .collect::<Vec<_>>();
            if kept.is_empty() {
                continue;
            }
            let prefix = &line[..line.find('{').unwrap_or(0)];
            out.push_str(prefix);
            out.push_str("{ ");
            out.push_str(&kept.join(", "));
            out.push_str(" } from");
            out.push_str(right);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

fn source_matches_disk(code: &str, source_file: Option<&str>) -> Option<std::path::PathBuf> {
    let source_file = source_file?;
    let disk_code = std::fs::read_to_string(source_file).ok()?;
    if disk_code != code {
        return None;
    }
    Some(
        std::fs::canonicalize(source_file)
            .unwrap_or_else(|_| std::path::PathBuf::from(source_file)),
    )
}

fn materialization_entry_is_ignored(
    source_root: &std::path::Path,
    source: &std::path::Path,
    name: &std::ffi::OsStr,
) -> bool {
    if matches!(
        name.to_str(),
        Some(
            ".git"
                | ".hg"
                | ".svn"
                | "target"
                | "node_modules"
                | "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".ruff_cache"
                | "dist"
                | "build"
        )
    ) {
        return true;
    }
    name == std::ffi::OsStr::new("results")
        && source.strip_prefix(source_root).ok() == Some(std::path::Path::new("bench"))
}

fn copy_materialization_tree(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    let source_root = std::fs::canonicalize(source)?;
    let mut active_directories = HashSet::new();
    copy_materialization_tree_inner(source, destination, &source_root, &mut active_directories)
}

fn copy_materialization_tree_inner(
    source: &std::path::Path,
    destination: &std::path::Path,
    source_root: &std::path::Path,
    active_directories: &mut HashSet<std::path::PathBuf>,
) -> std::io::Result<()> {
    let canonical_source = std::fs::canonicalize(source)?;
    if !canonical_source.starts_with(source_root)
        || !active_directories.insert(canonical_source.clone())
    {
        return Ok(());
    }
    let result = (|| {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            if materialization_entry_is_ignored(source_root, &canonical_source, &entry.file_name())
            {
                continue;
            }
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                let resolved = std::fs::canonicalize(&source_path)?;
                if !resolved.starts_with(source_root) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "materialization symlink escapes source root: {}",
                            source_path.display()
                        ),
                    ));
                }
                if resolved.is_dir() {
                    copy_materialization_tree_inner(
                        &resolved,
                        &destination_path,
                        source_root,
                        active_directories,
                    )?;
                } else if resolved.is_file() {
                    std::fs::copy(resolved, destination_path)?;
                }
            } else if file_type.is_dir() {
                copy_materialization_tree_inner(
                    &source_path,
                    &destination_path,
                    source_root,
                    active_directories,
                )?;
            } else if file_type.is_file() {
                std::fs::copy(&source_path, destination_path)?;
            }
        }
        Ok(())
    })();
    active_directories.remove(&canonical_source);
    result
}
struct NetworkGuard {
    _directory: tempfile::TempDir,
    python_sitecustomize: std::path::PathBuf,
    node_preload: std::path::PathBuf,
}

fn create_network_guard() -> Result<NetworkGuard, String> {
    let directory =
        tempfile::tempdir().map_err(|error| format!("failed to create network guard: {error}"))?;
    let python_sitecustomize = directory.path().join("sitecustomize.py");
    let node_preload = directory.path().join("network-guard.cjs");
    std::fs::write(
        &python_sitecustomize,
        r#"
import asyncio
import os
import socket
import subprocess

_NETWORK_MESSAGE = "court-jester network access denied"
_PROCESS_MESSAGE = "court-jester process spawn denied"

def _deny_network(*_args, **_kwargs):
    raise PermissionError(_NETWORK_MESSAGE)

def _deny_process(*_args, **_kwargs):
    raise PermissionError(_PROCESS_MESSAGE)

for _name in (
    "connect", "connect_ex", "send", "sendall", "sendto", "sendmsg",
    "setpeername", "getpeername",
):
    if hasattr(socket.socket, _name):
        setattr(socket.socket, _name, _deny_network)
for _name in (
    "create_connection", "create_server", "getaddrinfo", "gethostbyname",
    "gethostbyname_ex", "gethostbyaddr", "getnameinfo",
):
    if hasattr(socket, _name):
        setattr(socket, _name, _deny_network)
for _name in ("create_connection", "create_datagram_endpoint", "create_server"):
    if hasattr(asyncio.BaseEventLoop, _name):
        setattr(asyncio.BaseEventLoop, _name, _deny_network)
for _name in ("sock_connect", "sock_sendall", "sock_sendto", "sock_recv", "sock_recv_into"):
    if hasattr(asyncio.BaseEventLoop, _name):
        setattr(asyncio.BaseEventLoop, _name, _deny_network)
for _name in (
    "open_connection", "start_server", "create_connection",
    "create_datagram_endpoint", "getaddrinfo",
):
    if hasattr(asyncio, _name):
        setattr(asyncio, _name, _deny_network)
for _name in (
    "Popen", "run", "call", "check_call", "check_output", "getoutput",
    "getstatusoutput",
):
    if hasattr(subprocess, _name):
        setattr(subprocess, _name, _deny_process)
for _name in dir(os):
    if _name == "system" or _name == "popen" or _name.startswith("spawn") or _name.startswith("exec"):
        try:
            setattr(os, _name, _deny_process)
        except (AttributeError, TypeError):
            pass
"#,
    )
    .map_err(|error| format!("failed to write Python network guard: {error}"))?;
    std::fs::write(
        &node_preload,
        r#"
"use strict";
const networkMessage = "court-jester network access denied";
const processMessage = "court-jester process spawn denied";
const denyNetwork = () => { throw new Error(networkMessage); };
const denyProcess = () => { throw new Error(processMessage); };
function patch(object, names, replacement) {
  if (!object) return;
  for (const name of names) {
    if (typeof object[name] === "function") object[name] = replacement;
  }
}
for (const moduleName of ["net", "tls", "dgram", "dns", "http", "https"]) {
  try {
    const module = require(moduleName);
    patch(module, [
      "connect", "createConnection", "createServer", "createSocket",
      "lookup", "lookupService", "resolve", "resolve4", "resolve6",
      "resolveAny", "resolveCaa", "resolveCname", "resolveMx",
      "resolveNaptr", "resolveNs", "resolvePtr", "resolveSoa",
      "resolveSrv", "resolveTxt", "reverse", "request", "get",
    ], denyNetwork);
    if (module.promises) {
      patch(module.promises, [
        "lookup", "lookupService", "resolve", "resolve4", "resolve6",
        "resolveAny", "resolveCaa", "resolveCname", "resolveMx",
        "resolveNaptr", "resolveNs", "resolvePtr", "resolveSoa",
        "resolveSrv", "resolveTxt", "reverse",
      ], denyNetwork);
    }
  } catch (_) {}
}
try {
  const childProcess = require("child_process");
  patch(childProcess, [
    "spawn", "spawnSync", "exec", "execSync", "execFile", "execFileSync",
    "fork",
  ], denyProcess);
} catch (_) {}
try {
  const builtinModule = require("module");
  if (typeof builtinModule.syncBuiltinESMExports === "function") {
    builtinModule.syncBuiltinESMExports();
  }
} catch (_) {}
try {
  if (globalThis.Bun) {
    for (const name of ["spawn", "spawnSync", "connect", "listen", "serve"]) {
      if (typeof globalThis.Bun[name] === "function") {
        globalThis.Bun[name] = name === "spawn" || name === "spawnSync" ? denyProcess : denyNetwork;
      }
    }
    if (typeof globalThis.Bun.fetch === "function") globalThis.Bun.fetch = denyNetwork;
  }
} catch (_) {}
try {
  const workerThreads = require("worker_threads");
  if (workerThreads.Worker && workerThreads.Worker.prototype) {
    patch(workerThreads.Worker.prototype, ["postMessage"], denyProcess);
  }
} catch (_) {}
globalThis.fetch = denyNetwork;
globalThis.WebSocket = function CourtJesterBlockedWebSocket() { denyNetwork(); };
globalThis.__COURT_JESTER_NETWORK_GUARD__ = true;
"#,
    )
    .map_err(|error| format!("failed to write Node network guard: {error}"))?;
    Ok(NetworkGuard {
        _directory: directory,
        python_sitecustomize,
        node_preload,
    })
}

fn prepend_env_value(env: &mut Vec<(String, String)>, key: &str, prefix: &str) {
    if let Some((_, value)) = env.iter_mut().find(|(name, _)| name == key) {
        if value.is_empty() {
            *value = prefix.to_string();
        } else {
            *value = format!("{prefix}:{}", value);
        }
    } else {
        env.push((key.to_string(), prefix.to_string()));
    }
}

fn apply_network_guard(env: &mut Vec<(String, String)>, language: &Language, guard: &NetworkGuard) {
    match language {
        Language::Python => {
            prepend_env_value(
                env,
                "PYTHONPATH",
                guard
                    .python_sitecustomize
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_string_lossy()
                    .as_ref(),
            );
        }
        Language::TypeScript => {
            let preload = format!("--require={}", guard.node_preload.display());
            if let Some((_, value)) = env.iter_mut().find(|(name, _)| name == "NODE_OPTIONS") {
                *value = format!("{preload} {value}");
            } else {
                env.push(("NODE_OPTIONS".into(), preload));
            }
        }
    }
}

/// Walk up from a directory to find the Python package root.
/// Returns the deepest ancestor that has __init__.py in it,
/// plus the path from there to the starting dir.
fn find_python_package_root(start_dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    // Walk up while __init__.py exists
    let mut dir = start_dir.to_path_buf();
    let mut parts: Vec<String> = vec![];

    loop {
        if !dir.join("__init__.py").exists() {
            break;
        }
        parts.push(dir.file_name()?.to_str()?.to_string());
        dir = dir.parent()?.to_path_buf();
    }

    if parts.is_empty() {
        return None;
    }

    parts.reverse();
    let module_prefix = parts.join(".");
    // dir is now the parent of the package root
    Some((dir, module_prefix))
}

fn module_run_for_python_source(
    source_path: &std::path::Path,
) -> Option<(std::path::PathBuf, String)> {
    let parent = source_path.parent()?;
    let stem = source_path.file_stem()?.to_str()?;
    if !is_valid_python_module_name(stem) {
        return None;
    }

    let (pkg_root_parent, module_prefix) = find_python_package_root(parent)?;
    Some((pkg_root_parent, format!("{module_prefix}.{stem}")))
}

pub async fn docker_daemon_ready() -> Result<(), String> {
    let output = docker_output(&["info"]).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub async fn docker_image_id(image: &str) -> Result<String, String> {
    if image.trim().is_empty() || image.starts_with('-') {
        return Err("docker image must be non-empty and must not begin with '-'".into());
    }
    let output = docker_output(&["image", "inspect", image]).await?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v.get(0)?.get("Id")?.as_str().map(str::to_owned))
        .ok_or_else(|| "docker image inspect returned no image id".into())
}

async fn docker_output(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("docker")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("docker unavailable: {e}"))
}

/// Execute code using the shared context, materialization, and harness launcher.
pub async fn execute(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_standalone(code, language, options, None).await
}

async fn execute_standalone(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
    runtime_override: Option<HarnessRuntime>,
) -> ExecutionResult {
    if let Err(message) = options.validate() {
        return launch_failure(message);
    }
    let invocation_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            return launch_failure(format!("cannot resolve invocation directory: {error}"))
        }
    };
    let standalone_root = if options.project_dir.is_none() && options.source_file.is_none() {
        Some(match tempfile::tempdir() {
            Ok(root) => root,
            Err(error) => {
                return launch_failure(format!("failed to create standalone workspace: {error}"));
            }
        })
    } else {
        None
    };
    let context = if let Some(root) = standalone_root.as_ref() {
        let root = root.path().to_path_buf();
        ExecutionContext {
            invocation_dir: root.clone(),
            workspace_root: root.clone(),
            target_package_root: root,
            test_package_root: None,
            dependency_roots: Vec::new(),
            target_source: SourceContext {
                language: *language,
                mode: if *language == Language::Python {
                    SourceMode::Python
                } else {
                    SourceMode::TypeScript
                },
                source_file: None,
                virtual_file_path: None,
            },
            test_source: None,
        }
    } else {
        match crate::resolve_execution_context(crate::types::ContextRequest {
            invocation_dir: &invocation_dir,
            explicit_project_dir: options.project_dir.map(std::path::Path::new),
            target_file: options.source_file.map(std::path::Path::new),
            test_file: None,
            language: *language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return launch_failure(format!("cannot resolve execution context: {error}"));
            }
        }
    };
    let target_path = context.target_source.source_file.as_deref();
    let target_path_string = target_path.and_then(|path| path.to_str());
    let mode = context.target_source.mode;
    let extension = match mode {
        SourceMode::Python => "py",
        SourceMode::TypeScript => "ts",
        SourceMode::Tsx => "tsx",
    };
    let relative_entry = target_path
        .and_then(|path| path.strip_prefix(&context.workspace_root).ok())
        .filter(|path| !path.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from(format!(".court-jester-execute.{extension}")));
    let executable_code = if *language == Language::TypeScript {
        strip_typescript_type_only_imports(code, target_path_string)
    } else {
        code.to_string()
    };
    let existing = target_path_string
        .filter(|path| source_matches_disk(&executable_code, Some(path)).is_some())
        .is_some();
    let artifact = if existing {
        HarnessArtifact::Existing {
            relative_path: relative_entry,
        }
    } else {
        HarnessArtifact::Generated {
            code: executable_code,
            relative_path: relative_entry,
        }
    };
    let runtime = runtime_override.unwrap_or_else(|| match language {
        Language::Python => HarnessRuntime::Python,
        Language::TypeScript => {
            let source = target_path.and_then(|path| path.to_str());
            let repo_runner =
                detect_repo_typescript_runner(context.workspace_root.to_str(), source);
            let path = std::env::var("PATH").unwrap_or_default();
            if repo_runner.as_deref() == Some("bun") {
                HarnessRuntime::BunScript
            } else if which_binary(&path, "node").is_some() {
                HarnessRuntime::NodeScript
            } else {
                HarnessRuntime::BunScript
            }
        }
    });
    let harness = HarnessSpec {
        kind: HarnessKind::Standalone,
        runtime,
        test_adapter: None,
        source_mode: mode,
        artifact,
        args: Vec::new(),
        network: options.network_policy,
    };
    let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
    let source_file_owned = target_path.map(|path| path.to_string_lossy().into_owned());
    let limits = SandboxOptions {
        timeout_seconds: options.timeout_seconds,
        memory_mb: options.memory_mb,
        runtime_profile: options.runtime_profile,
        network_policy: options.network_policy,
        harness_args: options.harness_args,
        docker_image: options.docker_image,
        project_dir: Some(project_dir_owned.as_str()),
        source_file: source_file_owned.as_deref(),
    };
    execute_harness(&context, harness, limits).await.process
}

pub async fn execute_typescript_node(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::NodeScript),
    )
    .await
}

pub async fn execute_typescript_bun(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::BunScript),
    )
    .await
}

pub async fn execute_typescript_bun_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::BunTest),
    )
    .await
}

pub async fn execute_typescript_repo_native(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_standalone(
            code,
            &Language::TypeScript,
            options,
            Some(HarnessRuntime::BunScript),
        )
        .await,
    )
}

pub async fn execute_typescript_repo_native_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_standalone(
            code,
            &Language::TypeScript,
            options,
            Some(HarnessRuntime::BunTest),
        )
        .await,
    )
}

fn normalize_harness_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    if path.is_absolute() {
        return Err("harness artifact paths must be relative".into());
    }
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("harness artifact path escapes its overlay".into())
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("harness artifact path must not be empty".into());
    }
    if normalized.to_string_lossy().contains('\0') {
        return Err("harness artifact path contains NUL".into());
    }
    Ok(normalized)
}

fn harness_extension_compatible(path: &std::path::Path, mode: SourceMode) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match mode {
        SourceMode::Python => extension.as_deref() == Some("py"),
        SourceMode::TypeScript => matches!(extension.as_deref(), Some("ts") | Some("js")),
        SourceMode::Tsx => matches!(extension.as_deref(), Some("tsx") | Some("jsx")),
    }
}

fn resolve_harness_arg(
    argument: &HarnessArg,
    workspace_root: &std::path::Path,
) -> Result<std::ffi::OsString, String> {
    match argument {
        HarnessArg::Literal { literal } => {
            if literal.contains('\0') {
                Err("harness literal argument contains NUL".into())
            } else {
                Ok(literal.clone().into())
            }
        }
        HarnessArg::ProjectPath { project_path } => {
            let path = std::path::Path::new(project_path);
            if path.is_absolute() {
                return Err("project_path arguments must be relative".into());
            }
            let normalized = normalize_harness_path(path)?;
            let joined = workspace_root.join(normalized);
            let canonical = std::fs::canonicalize(&joined).map_err(|error| {
                format!("project_path '{}' is unavailable: {error}", project_path)
            })?;
            if !canonical.starts_with(workspace_root) {
                return Err(format!(
                    "project_path '{}' escapes workspace root",
                    project_path
                ));
            }
            Ok(canonical.into_os_string())
        }
    }
}

fn launch_failure(message: impl Into<String>) -> ExecutionResult {
    let message = message.into();
    let termination = termination(ProcessTerminationKind::LaunchFailed, None, None);
    ExecutionResult {
        stdout: String::new(),
        stderr: message.clone(),
        exit_code: None,
        duration_ms: 0,
        timed_out: false,
        memory_error: false,
        termination: Some(termination.clone()),
        diagnostics: vec![FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::LauncherFailure,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message,
            process: Some(termination),
            limits: None,
        }],
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_command_with_limits(
    mut command: Command,
    timeout_seconds: f64,
    memory_mb: u64,
    runtime_profile: RuntimeProfile,
    network_policy: NetworkPolicy,
    is_typescript: bool,
    launch_error_prefix: &str,
) -> ExecutionResult {
    if !timeout_seconds.is_finite() || timeout_seconds <= 0.0 {
        return launch_failure("timeout must be finite and greater than zero");
    }
    let Some(memory_bytes) = memory_mb
        .checked_mul(1024)
        .and_then(|value| value.checked_mul(1024))
    else {
        return launch_failure("memory limit is too large");
    };

    let cpu_secs = timeout_seconds.ceil().max(1.0) as u64;
    unsafe {
        command.pre_exec(move || {
            use nix::sys::resource::{setrlimit, Resource};
            libc::setsid();
            if !is_typescript {
                let _ = setrlimit(Resource::RLIMIT_AS, memory_bytes, memory_bytes);
                let _ = setrlimit(Resource::RLIMIT_DATA, memory_bytes, memory_bytes);
            }
            let _ = setrlimit(Resource::RLIMIT_CPU, cpu_secs, cpu_secs);
            let ten_mb = 10 * 1024 * 1024;
            let _ = setrlimit(Resource::RLIMIT_FSIZE, ten_mb, ten_mb);
            Ok(())
        });
    }

    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return launch_failure(format!("{launch_error_prefix}: {error}"));
        }
    };
    let pid = child.id().unwrap_or_default();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut pipe, &mut bytes).await;
        }
        bytes
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut pipe, &mut bytes).await;
        }
        bytes
    });
    let memory_killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let memory_killed_clone = memory_killed.clone();
    let monitor = (pid > 0).then(|| {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if get_process_group_rss_bytes(pid) > memory_bytes {
                    memory_killed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    break;
                }
            }
        })
    });

    let mut wait = Box::pin(child.wait());
    let mut timeout = Box::pin(tokio::time::sleep(std::time::Duration::from_secs_f64(
        timeout_seconds,
    )));
    let mut timed_out = false;
    let wait_result = tokio::select! {
        result = &mut wait => result,
        _ = &mut timeout => {
            timed_out = true;
            if pid > 0 {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            wait.await
        }
    };
    if let Some(handle) = monitor {
        handle.abort();
    }

    let stdout = String::from_utf8_lossy(&stdout_task.await.unwrap_or_default()).to_string();
    let mut stderr = String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).to_string();
    let duration_ms = started.elapsed().as_millis() as u64;
    let (status, wait_error) = match wait_result {
        Ok(status) => (Some(status), None),
        Err(error) => (None, Some(error)),
    };
    if let Some(error) = wait_error.as_ref() {
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&format!("failed to wait for process: {error}"));
    }
    let signal = status.as_ref().and_then(status_signal);
    let memory_limited = memory_killed.load(std::sync::atomic::Ordering::SeqCst);
    let kind = if memory_limited {
        ProcessTerminationKind::MemoryLimit
    } else if timed_out || signal == Some(libc::SIGXCPU) {
        ProcessTerminationKind::TimedOut
    } else if wait_error.is_some() {
        ProcessTerminationKind::WaitFailed
    } else if signal.is_some() {
        ProcessTerminationKind::Signaled
    } else {
        ProcessTerminationKind::Exited
    };
    let termination = termination(
        kind,
        status.as_ref().and_then(std::process::ExitStatus::code),
        signal,
    );
    if memory_limited && stderr.is_empty() {
        stderr = format!("Killed: memory limit exceeded ({memory_mb} MB)");
    } else if matches!(kind, ProcessTerminationKind::TimedOut) && stderr.is_empty() {
        stderr = "Process timed out".into();
    }
    let limits = ExecutionLimits {
        timeout_seconds,
        memory_mb,
        runtime_profile,
        network_policy,
    };
    let mut diagnostics = match kind {
        ProcessTerminationKind::MemoryLimit => vec![FailureDiagnostic {
            domain: FailureDomain::Resource,
            kind: FailureKind::MemoryLimit,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: stderr.clone(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        }],
        ProcessTerminationKind::TimedOut => vec![FailureDiagnostic {
            domain: FailureDomain::Resource,
            kind: FailureKind::Timeout,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: stderr.clone(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        }],
        ProcessTerminationKind::LaunchFailed | ProcessTerminationKind::WaitFailed => {
            vec![FailureDiagnostic {
                domain: FailureDomain::Environment,
                kind: if matches!(kind, ProcessTerminationKind::LaunchFailed) {
                    FailureKind::LauncherFailure
                } else {
                    FailureKind::ToolFailure
                },
                component: DiagnosticComponent::Sandbox,
                impact: DiagnosticImpact::Blocking,
                message: stderr.clone(),
                process: Some(termination.clone()),
                limits: Some(limits.clone()),
            }]
        }
        _ => Vec::new(),
    };
    if network_policy == NetworkPolicy::Deny
        && stderr.contains("court-jester network access denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network access was denied by the sandbox".into(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        });
    }
    if network_policy == NetworkPolicy::Deny && stderr.contains("court-jester process spawn denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ProcessSpawnDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "process spawning was denied by the sandbox".into(),
            process: Some(termination.clone()),
            limits: Some(limits),
        });
    }
    ExecutionResult {
        stdout,
        stderr,
        exit_code: status.as_ref().and_then(std::process::ExitStatus::code),
        duration_ms,
        timed_out: matches!(kind, ProcessTerminationKind::TimedOut),
        memory_error: matches!(kind, ProcessTerminationKind::MemoryLimit),
        termination: Some(termination),
        diagnostics,
    }
}

async fn run_launch_command(
    plan: &LaunchPlan,
    timeout_seconds: f64,
    memory_mb: u64,
    runtime_profile: RuntimeProfile,
    network_policy: NetworkPolicy,
    is_typescript: bool,
) -> ExecutionResult {
    let configured_path = plan
        .env
        .iter()
        .find(|(name, _)| name == std::ffi::OsStr::new("PATH"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
    let mut command = Command::new(&plan.executable);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .env("PATH", configured_path)
        .env("HOME", plan.cwd.to_string_lossy().as_ref())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for (key, value) in &plan.env {
        command.env(key, value);
    }
    run_command_with_limits(
        command,
        timeout_seconds,
        memory_mb,
        runtime_profile,
        network_policy,
        is_typescript,
        "failed to launch harness",
    )
    .await
}

fn harness_diagnostics(
    adapter: Option<TestAdapter>,
    process: &ExecutionResult,
    limits: &ExecutionLimits,
) -> Vec<FailureDiagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(termination) = process.termination.as_ref() {
        let (domain, kind, message) = match termination.kind {
            ProcessTerminationKind::TimedOut => (
                FailureDomain::Resource,
                FailureKind::Timeout,
                "harness timed out".to_string(),
            ),
            ProcessTerminationKind::MemoryLimit => (
                FailureDomain::Resource,
                FailureKind::MemoryLimit,
                "harness exceeded the memory limit".to_string(),
            ),
            ProcessTerminationKind::Signaled => (
                FailureDomain::Environment,
                FailureKind::Signal,
                termination
                    .signal_name
                    .clone()
                    .unwrap_or_else(|| "harness terminated by signal".into()),
            ),
            ProcessTerminationKind::LaunchFailed => (
                FailureDomain::Environment,
                FailureKind::LauncherFailure,
                process.stderr.clone(),
            ),
            ProcessTerminationKind::WaitFailed => (
                FailureDomain::Environment,
                FailureKind::ToolFailure,
                process.stderr.clone(),
            ),
            ProcessTerminationKind::Exited => {
                if process.exit_code == Some(0) {
                    (
                        FailureDomain::Environment,
                        FailureKind::ToolFailure,
                        String::new(),
                    )
                } else {
                    (
                        FailureDomain::Environment,
                        FailureKind::NonzeroExit,
                        process.stderr.clone(),
                    )
                }
            }
        };
        if !message.is_empty() {
            diagnostics.push(FailureDiagnostic {
                domain,
                kind,
                component: DiagnosticComponent::Sandbox,
                impact: DiagnosticImpact::Blocking,
                message,
                process: Some(termination.clone()),
                limits: Some(limits.clone()),
            });
        }
    }

    if limits.network_policy == NetworkPolicy::Deny
        && process
            .stderr
            .contains("court-jester network access denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network access was denied by the harness sandbox".into(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    if limits.network_policy == NetworkPolicy::Deny
        && process.stderr.contains("court-jester process spawn denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ProcessSpawnDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "process spawning was denied by the harness sandbox".into(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    if process.exit_code == Some(0) {
        return diagnostics;
    }
    match adapter.unwrap_or(TestAdapter::Opaque) {
        TestAdapter::Opaque => {}
        TestAdapter::NodeTap => {
            if process.stdout.contains("not ok") || process.stderr.contains("not ok") {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::TargetCode,
                    kind: FailureKind::AssertionFailure,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Gating,
                    message: "authoritative TAP test failed".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            } else if !process.stdout.contains("TAP version") {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::VerifierHarness,
                    kind: FailureKind::HarnessProtocol,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Blocking,
                    message: "Node test runner did not emit valid TAP output".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            }
        }
        TestAdapter::BunJunit | TestAdapter::VitestJson | TestAdapter::JestJson => {
            let structured_failure = process.stdout.contains("<failure")
                || process.stdout.contains("\"status\":\"failed\"")
                || process.stdout.contains("\"pass\":false")
                || process.stderr.contains("<failure");
            if structured_failure {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::TargetCode,
                    kind: FailureKind::AssertionFailure,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Gating,
                    message: "authoritative test assertion failed".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            } else {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::VerifierHarness,
                    kind: FailureKind::HarnessProtocol,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Blocking,
                    message: "test runner did not emit a recognized structured result".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            }
        }
    }
    diagnostics
}

#[allow(clippy::too_many_arguments)]
async fn run_harness_in_docker(
    root: Option<&std::path::Path>,
    host_artifact: &std::path::Path,
    launch_cwd: &std::path::Path,
    context: &crate::types::ExecutionContext,
    harness: &crate::types::HarnessSpec,
    limits: crate::types::SandboxOptions<'_>,
) -> ExecutionResult {
    let image = limits.docker_image.unwrap_or_default();
    let started = Instant::now();
    match docker_output(&["image", "inspect", image]).await {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return launch_failure(format!(
                "docker image inspect failed for {image}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(error) => return launch_failure(format!("docker setup failed: {error}")),
    }

    let mirror = if root.is_none() {
        match tempfile::tempdir() {
            Ok(directory) => Some(directory),
            Err(error) => {
                return launch_failure(format!(
                    "docker setup failed creating project mirror: {error}"
                ));
            }
        }
    } else {
        None
    };
    if let Some(mirror) = mirror.as_ref() {
        if let Err(error) = copy_materialization_tree(&context.workspace_root, mirror.path()) {
            return launch_failure(format!(
                "docker setup failed materializing project mirror: {error}"
            ));
        }
    }
    let source_root = root
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| context.workspace_root.clone());
    let source_root = std::fs::canonicalize(&source_root).unwrap_or_else(|_| source_root.clone());
    let project_root = root
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        .or_else(|| {
            mirror
                .as_ref()
                .map(|directory| directory.path().to_path_buf())
        })
        .unwrap_or_else(|| source_root.clone());
    let artifact_relative = match host_artifact.strip_prefix(&source_root) {
        Ok(path) if !path.as_os_str().is_empty() => path.to_path_buf(),
        _ => return launch_failure("docker harness artifact is outside the project mirror"),
    };
    let artifact_relative = match normalize_harness_path(&artifact_relative) {
        Ok(path) => path,
        Err(error) => return launch_failure(error),
    };
    let container_artifact = std::path::Path::new("/workspace").join(&artifact_relative);
    let cwd_relative = launch_cwd
        .strip_prefix(&source_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let container_cwd = std::path::Path::new("/workspace").join(cwd_relative);

    let container = format!(
        "court-jester-harness-{}-{}",
        std::process::id(),
        started.elapsed().as_nanos()
    );
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let mut create = vec![
        "create".to_string(),
        "--name".to_string(),
        container.clone(),
        "--pull=never".to_string(),
        "--network".to_string(),
        "none".to_string(),
        "--cpus".to_string(),
        "1".to_string(),
        "--memory".to_string(),
        format!("{}m", limits.memory_mb),
        "--memory-swap".to_string(),
        format!("{}m", limits.memory_mb),
        "--pids-limit".to_string(),
        "128".to_string(),
        "--read-only".to_string(),
        "--tmpfs".to_string(),
        "/tmp:rw,nosuid,nodev,noexec,size=64m".to_string(),
        "--ulimit".to_string(),
        "fsize=10485760:10485760".to_string(),
        "--user".to_string(),
        format!("{uid}:{gid}"),
        "-e".to_string(),
        "HOME=/tmp".to_string(),
        "-e".to_string(),
        "PYTHONDONTWRITEBYTECODE=1".to_string(),
        "--mount".to_string(),
        format!(
            "type=bind,src={},dst=/workspace,readonly",
            project_root.display()
        ),
        "--workdir".to_string(),
        container_cwd.to_string_lossy().into_owned(),
        image.to_string(),
    ];

    let mut python_paths = Vec::new();
    let mut node_paths = Vec::new();
    for (index, dependency) in context.dependency_roots.iter().enumerate() {
        let canonical = match std::fs::canonicalize(dependency) {
            Ok(path) if path.is_dir() => path,
            _ => continue,
        };
        if let Ok(relative) = canonical.strip_prefix(&project_root) {
            let container_path = std::path::Path::new("/workspace").join(relative);
            python_paths.push(container_path.to_string_lossy().into_owned());
            node_paths.push(container_path.to_string_lossy().into_owned());
            continue;
        }
        let container_path =
            std::path::PathBuf::from(format!("/court-jester/dependencies/{index}"));
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst={},readonly",
                canonical.display(),
                container_path.display()
            ),
        );
        let container_path = container_path.to_string_lossy().into_owned();
        python_paths.push(container_path.clone());
        node_paths.push(container_path);
    }
    if !python_paths.is_empty() {
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!("PYTHONPATH={}", python_paths.join(":")),
        );
    }
    if !node_paths.is_empty() {
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!("NODE_PATH={}", node_paths.join(":")),
        );
    }

    let artifact = container_artifact.to_string_lossy().into_owned();
    let mut command = match harness.runtime {
        crate::types::HarnessRuntime::Python => vec!["python3".to_string(), artifact],
        crate::types::HarnessRuntime::NodeScript => vec![
            "node".to_string(),
            "--no-warnings".to_string(),
            "--experimental-transform-types".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::TsxScript => vec!["tsx".to_string(), artifact],
        crate::types::HarnessRuntime::BunScript => {
            vec!["bun".to_string(), "run".to_string(), artifact]
        }
        crate::types::HarnessRuntime::NodeTest => vec![
            "node".to_string(),
            "--no-warnings".to_string(),
            "--experimental-transform-types".to_string(),
            "--test-reporter=tap".to_string(),
            "--test".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::BunTest => vec![
            "bun".to_string(),
            "test".to_string(),
            "--reporter=junit".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::Vitest => vec![
            "vitest".to_string(),
            "run".to_string(),
            "--reporter=json".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::Jest => {
            vec!["jest".to_string(), "--json".to_string(), artifact]
        }
        crate::types::HarnessRuntime::RepoTest => vec![
            "npm".to_string(),
            "test".to_string(),
            "--".to_string(),
            artifact,
        ],
    };
    for argument in harness.args.iter().chain(limits.harness_args.iter()) {
        match argument {
            crate::types::HarnessArg::Literal { literal } => {
                if literal.contains('\0') {
                    return launch_failure("harness literal argument contains NUL");
                }
                command.push(literal.clone());
            }
            crate::types::HarnessArg::ProjectPath { project_path } => {
                let relative = match normalize_harness_path(std::path::Path::new(project_path)) {
                    Ok(path) => path,
                    Err(error) => return launch_failure(error),
                };
                command.push(
                    std::path::Path::new("/workspace")
                        .join(relative)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    create.extend(command);
    let create_args: Vec<&str> = create.iter().map(String::as_str).collect();
    match docker_output(&create_args).await {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return launch_failure(format!(
                "docker create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(error) => return launch_failure(format!("docker create failed: {error}")),
    }
    let cleanup = || async {
        let _ = docker_output(&["rm", "-f", &container]).await;
    };
    match docker_output(&["start", &container]).await {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            cleanup().await;
            return launch_failure(format!(
                "docker start failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(error) => {
            cleanup().await;
            return launch_failure(format!("docker start failed: {error}"));
        }
    }
    let wait_result = tokio::time::timeout(
        std::time::Duration::from_secs_f64(limits.timeout_seconds),
        docker_output(&["wait", &container]),
    )
    .await;
    let timed_out = wait_result.is_err();
    if timed_out {
        let _ = docker_output(&["kill", &container]).await;
    }
    let state_output = docker_output(&["inspect", "--format", "{{json .State}}", &container])
        .await
        .ok();
    let logs = docker_output(&["logs", &container]).await.ok();
    let state = state_output
        .as_ref()
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
    let memory_limited = state
        .as_ref()
        .and_then(|value| value.get("OOMKilled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let exit_code = state
        .as_ref()
        .and_then(|value| value.get("ExitCode"))
        .and_then(serde_json::Value::as_i64)
        .map(|value| value as i32);
    let stdout = logs
        .as_ref()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let stderr = logs
        .as_ref()
        .map(|output| String::from_utf8_lossy(&output.stderr).to_string())
        .unwrap_or_default();
    cleanup().await;
    let wait_failed = match &wait_result {
        Ok(Ok(output)) => !output.status.success(),
        Ok(Err(_)) | Err(_) => true,
    } || state.is_none();
    let kind = if timed_out {
        ProcessTerminationKind::TimedOut
    } else if memory_limited {
        ProcessTerminationKind::MemoryLimit
    } else if wait_failed {
        ProcessTerminationKind::WaitFailed
    } else {
        ProcessTerminationKind::Exited
    };
    let termination = termination(kind, if timed_out { None } else { exit_code }, None);
    let mut process = ExecutionResult {
        stdout,
        stderr: if timed_out {
            "Process timed out".into()
        } else if memory_limited {
            format!("Killed: memory limit exceeded ({} MB)", limits.memory_mb)
        } else if wait_failed && stderr.is_empty() {
            "docker wait or inspect failed".into()
        } else {
            stderr
        },
        exit_code: if timed_out { None } else { exit_code },
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        memory_error: memory_limited,
        termination: Some(termination),
        diagnostics: Vec::new(),
    };
    let execution_limits = crate::types::ExecutionLimits {
        timeout_seconds: limits.timeout_seconds,
        memory_mb: limits.memory_mb,
        runtime_profile: limits.runtime_profile,
        network_policy: NetworkPolicy::Deny,
    };
    process.diagnostics = harness_diagnostics(harness.test_adapter, &process, &execution_limits);
    process
}

fn virtual_env_bin(virtual_env: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    virtual_env.map(|root| {
        std::path::PathBuf::from(root).join(if cfg!(windows) { "Scripts" } else { "bin" })
    })
}

pub async fn execute_harness(
    context: &ExecutionContext,
    harness: HarnessSpec,
    limits: SandboxOptions<'_>,
) -> HarnessExecution {
    if let Err(error) = limits.validate() {
        let process = launch_failure(error);
        return HarnessExecution {
            diagnostics: process.diagnostics.clone(),
            process,
        };
    }
    let effective_network =
        if limits.network_policy == NetworkPolicy::Deny || harness.network == NetworkPolicy::Deny {
            NetworkPolicy::Deny
        } else {
            NetworkPolicy::Allow
        };
    if limits.runtime_profile == RuntimeProfile::Isolated
        && effective_network == NetworkPolicy::Allow
    {
        let process = launch_failure("isolated harnesses cannot enable network access");
        let diagnostic = FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network allow is incompatible with isolated execution".into(),
            process: None,
            limits: Some(ExecutionLimits {
                timeout_seconds: limits.timeout_seconds,
                memory_mb: limits.memory_mb,
                runtime_profile: limits.runtime_profile,
                network_policy: effective_network,
            }),
        };
        return HarnessExecution {
            process,
            diagnostics: vec![diagnostic],
        };
    }

    let (temporary, host_artifact, mut launch_cwd) = match harness.artifact.clone() {
        HarnessArtifact::Generated {
            code,
            relative_path,
        } => {
            let relative_path = match normalize_harness_path(&relative_path) {
                Ok(path) => path,
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            if !harness_extension_compatible(&relative_path, harness.source_mode) {
                let process =
                    launch_failure("harness artifact extension is incompatible with source mode");
                return HarnessExecution {
                    diagnostics: vec![FailureDiagnostic {
                        domain: FailureDomain::VerifierHarness,
                        kind: FailureKind::UnsupportedSourceMode,
                        component: DiagnosticComponent::FuzzHarness,
                        impact: DiagnosticImpact::Blocking,
                        message: "harness artifact extension is incompatible with source mode"
                            .into(),
                        process: None,
                        limits: None,
                    }],
                    process,
                };
            }
            let temporary = match tempfile::tempdir() {
                Ok(directory) => directory,
                Err(error) => {
                    let process =
                        launch_failure(format!("failed to create harness overlay: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            let package_overlay = if harness.source_mode == SourceMode::Python
                && context.workspace_root.join("__init__.py").is_file()
            {
                context
                    .workspace_root
                    .file_name()
                    .map(|name| name.to_os_string())
                    .filter(|name| is_valid_python_module_name(&name.to_string_lossy()))
            } else {
                None
            };
            let overlay_root = package_overlay
                .as_ref()
                .map(|name| temporary.path().join(name))
                .unwrap_or_else(|| temporary.path().to_path_buf());
            let should_materialize = limits
                .source_file
                .is_some_and(|path| std::path::Path::new(path).is_file())
                || (harness.kind == HarnessKind::Standalone && limits.project_dir.is_some());
            if should_materialize {
                if let Err(error) =
                    copy_materialization_tree(&context.workspace_root, &overlay_root)
                {
                    let process =
                        launch_failure(format!("failed to materialize project overlay: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            }
            let host_artifact = overlay_root.join(&relative_path);
            if let Some(parent) = host_artifact.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    let process =
                        launch_failure(format!("failed to create harness directory: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            }
            let code = match harness.source_mode {
                crate::types::SourceMode::TypeScript | crate::types::SourceMode::Tsx => {
                    rewrite_typescript_relative_imports(
                        &code,
                        context.target_source.source_file.as_deref(),
                    )
                }
                crate::types::SourceMode::Python => code,
            };
            if let Err(error) = std::fs::write(&host_artifact, code) {
                let process = launch_failure(format!("failed to materialize harness: {error}"));
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            let package_relative = context
                .target_package_root
                .strip_prefix(&context.workspace_root)
                .unwrap_or_else(|_| std::path::Path::new(""));
            let launch_cwd = if package_overlay.is_some() {
                temporary.path().to_path_buf()
            } else {
                temporary.path().join(package_relative)
            };
            if let Err(error) = std::fs::create_dir_all(&launch_cwd) {
                let process = launch_failure(format!(
                    "failed to create harness working directory: {error}"
                ));
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            (Some(temporary), host_artifact, launch_cwd)
        }
        HarnessArtifact::Existing { relative_path } => {
            let relative_path = match normalize_harness_path(&relative_path) {
                Ok(path) => path,
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            if !harness_extension_compatible(&relative_path, harness.source_mode) {
                let process =
                    launch_failure("existing artifact extension is incompatible with source mode");
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            let host_artifact = context.workspace_root.join(&relative_path);
            let workspace_root = std::fs::canonicalize(&context.workspace_root)
                .unwrap_or_else(|_| context.workspace_root.clone());
            let canonical = match std::fs::canonicalize(&host_artifact) {
                Ok(path) if path.starts_with(&workspace_root) => path,
                Ok(_) => {
                    let process = launch_failure("existing harness escapes workspace root");
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
                Err(error) => {
                    let process =
                        launch_failure(format!("existing harness is unavailable: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            (None, canonical, context.target_package_root.clone())
        }
    };

    let path_env = {
        let base = std::env::var("PATH").unwrap_or_default();
        let mut prefixes = Vec::new();
        if let Some(bin) = virtual_env_bin(std::env::var_os("VIRTUAL_ENV").as_deref()) {
            prefixes.push(bin.to_string_lossy().into_owned());
        }
        for root in &context.dependency_roots {
            prefixes.push(
                root.join("node_modules/.bin")
                    .to_string_lossy()
                    .into_owned(),
            );
            prefixes.push(root.join(".venv/bin").to_string_lossy().into_owned());
        }
        if prefixes.is_empty() {
            base
        } else if base.is_empty() {
            prefixes.join(":")
        } else {
            format!("{}:{}", prefixes.join(":"), base)
        }
    };
    let executable_name = match harness.runtime {
        HarnessRuntime::Python => "python3",
        HarnessRuntime::NodeScript | HarnessRuntime::NodeTest | HarnessRuntime::TsxScript => {
            if harness.runtime == HarnessRuntime::TsxScript {
                "tsx"
            } else {
                "node"
            }
        }
        HarnessRuntime::BunScript | HarnessRuntime::BunTest => "bun",
        HarnessRuntime::Vitest => "vitest",
        HarnessRuntime::Jest => "jest",
        HarnessRuntime::RepoTest => "npm",
    };
    let executable = match which_binary(&path_env, executable_name) {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let process = launch_failure(format!(
                "required runtime '{executable_name}' is unavailable"
            ));
            return HarnessExecution {
                diagnostics: process.diagnostics.clone(),
                process,
            };
        }
    };
    let mut args = Vec::<std::ffi::OsString>::new();
    match harness.runtime {
        HarnessRuntime::Python => {}
        HarnessRuntime::NodeScript | HarnessRuntime::TsxScript => {
            if harness.runtime == HarnessRuntime::NodeScript {
                args.extend(["--no-warnings", "--experimental-transform-types"].map(Into::into));
            }
        }
        HarnessRuntime::BunScript => {}
        HarnessRuntime::NodeTest => {
            args.extend(
                [
                    "--no-warnings",
                    "--experimental-transform-types",
                    "--test-reporter=tap",
                    "--test",
                ]
                .map(Into::into),
            );
        }
        HarnessRuntime::BunTest => args.extend(["test", "--reporter=junit"].map(Into::into)),
        HarnessRuntime::Vitest => args.extend(["run", "--reporter=json"].map(Into::into)),
        HarnessRuntime::Jest => args.extend(["--json"].map(Into::into)),
        HarnessRuntime::RepoTest => args.extend(["test", "--"].map(Into::into)),
    }
    let python_module = matches!(harness.runtime, HarnessRuntime::Python)
        .then(|| module_run_for_python_source(&host_artifact))
        .flatten();
    if let Some((module_root, module_name)) = python_module {
        launch_cwd = module_root;
        args.extend(["-m".into(), module_name.into()]);
    } else {
        args.push(host_artifact.clone().into_os_string());
    }
    for argument in harness.args.iter().chain(limits.harness_args.iter()) {
        match resolve_harness_arg(argument, &context.workspace_root) {
            Ok(value) => args.push(value),
            Err(error) => {
                let process = launch_failure(error);
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
        }
    }
    let mut env = Vec::new();
    env.push(("PATH".into(), path_env.clone()));
    let dependency_paths = context
        .dependency_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if !dependency_paths.is_empty() {
        let joined = dependency_paths.join(if cfg!(windows) { ";" } else { ":" });
        env.push(("NODE_PATH".into(), joined.clone()));
        env.push(("PYTHONPATH".into(), joined));
    }
    let network_guard = if effective_network == NetworkPolicy::Deny {
        match create_network_guard() {
            Ok(guard) => Some(guard),
            Err(error) => {
                let process = launch_failure(error);
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
        }
    } else {
        None
    };
    if let Some(guard) = network_guard.as_ref() {
        match harness.source_mode {
            SourceMode::Python => apply_network_guard(&mut env, &Language::Python, guard),
            SourceMode::TypeScript | SourceMode::Tsx => {
                apply_network_guard(&mut env, &Language::TypeScript, guard)
            }
        }
        if matches!(
            harness.runtime,
            HarnessRuntime::BunScript | HarnessRuntime::BunTest
        ) {
            args.splice(
                0..0,
                [
                    std::ffi::OsString::from("--preload"),
                    guard.node_preload.clone().into_os_string(),
                ],
            );
            env.retain(|(name, _)| name != "NODE_OPTIONS");
        }
    }
    let plan = LaunchPlan {
        executable,
        args,
        cwd: launch_cwd,
        env: env
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
        host_artifact,
        container_artifact: None,
    };
    let execution_limits = ExecutionLimits {
        timeout_seconds: limits.timeout_seconds,
        memory_mb: limits.memory_mb,
        runtime_profile: limits.runtime_profile,
        network_policy: effective_network,
    };
    let is_typescript = !matches!(harness.source_mode, SourceMode::Python);
    let mut process = if limits.runtime_profile == RuntimeProfile::Isolated {
        run_harness_in_docker(
            temporary.as_ref().map(|directory| directory.path()),
            &plan.host_artifact,
            &plan.cwd,
            context,
            &harness,
            limits,
        )
        .await
    } else {
        run_launch_command(
            &plan,
            limits.timeout_seconds,
            limits.memory_mb,
            limits.runtime_profile,
            effective_network,
            is_typescript,
        )
        .await
    };
    let mut diagnostics = harness_diagnostics(harness.test_adapter, &process, &execution_limits);
    if matches!(harness.kind, HarnessKind::GeneratedVerifier)
        && process
            .termination
            .as_ref()
            .is_some_and(|termination| matches!(termination.kind, ProcessTerminationKind::Exited))
    {
        let combined = format!("{}\n{}", process.stdout, process.stderr);
        match parse_harness_events(&combined) {
            Ok(summary) if !summary.harness_completed => diagnostics.push(FailureDiagnostic {
                domain: FailureDomain::VerifierHarness,
                kind: FailureKind::HarnessProtocol,
                component: DiagnosticComponent::FuzzHarness,
                impact: DiagnosticImpact::Blocking,
                message: "generated verifier did not emit harness_completed".into(),
                process: process.termination.clone(),
                limits: Some(execution_limits.clone()),
            }),
            Err(error) => diagnostics.push(FailureDiagnostic {
                domain: FailureDomain::VerifierHarness,
                kind: FailureKind::HarnessProtocol,
                component: DiagnosticComponent::FuzzHarness,
                impact: DiagnosticImpact::Blocking,
                message: error,
                process: process.termination.clone(),
                limits: Some(execution_limits.clone()),
            }),
            _ => {}
        }
    }
    process.diagnostics = diagnostics.clone();
    drop(temporary);
    HarnessExecution {
        process,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        copy_materialization_tree, has_typescript_type_only_relative_imports, virtual_env_bin,
        which_binary,
    };

    #[test]
    fn which_binary_finds_existing_binary_on_path() {
        let path_env = "/missing:/bin:/usr/bin";
        assert_eq!(which_binary(path_env, "sh"), Some("/bin/sh".to_string()));
    }

    #[test]
    fn which_binary_returns_none_for_missing_binary() {
        let path_env = "/missing:/also-missing";
        assert_eq!(which_binary(path_env, "definitely-not-a-real-binary"), None);
    }

    #[test]
    fn detects_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(
            &helper_path,
            "export type PathValue = string | number;\nexport const value = 7;\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue, value } from \"./internals.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn ignores_plain_value_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("helper.ts");
        std::fs::write(&helper_path, "export const value = 7;\n").unwrap();
        std::fs::write(
            &source_path,
            "import { value } from \"./helper.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(!has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_multiline_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &source_path,
            "import {\n  PathValue,\n} from \"./internals.ts\";\nconsole.log('ok');\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_transitive_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("test.ts");
        let object_path = dir.path().join("object.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &object_path,
            "import { PathValue } from \"./internals.ts\";\nexport function pick(path: PathValue): string { return String(path); }\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { pick } from \"./object.ts\";\nconsole.log(pick(\"x\"));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_type_only_relative_reexports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export type { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_value_reexports_of_type_only_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn ignores_generated_benchmark_results() {
        let source = tempfile::tempdir().unwrap();
        let benchmark_dir = source.path().join("bench");
        let results_dir = benchmark_dir.join("results");
        std::fs::create_dir_all(&results_dir).unwrap();
        std::fs::write(source.path().join("keep.py"), "value = 1\n").unwrap();
        std::fs::write(benchmark_dir.join("keep.py"), "value = 2\n").unwrap();
        std::fs::write(results_dir.join("report.json"), "{}\n").unwrap();

        let destination = tempfile::tempdir().unwrap();
        copy_materialization_tree(source.path(), destination.path()).unwrap();

        assert!(destination.path().join("keep.py").is_file());
        assert!(destination.path().join("bench/keep.py").is_file());
        assert!(!destination.path().join("bench/results").exists());
    }

    #[test]
    fn active_virtual_environment_contributes_runtime_bin() {
        let root = std::path::Path::new("/tmp/project-environment");
        assert_eq!(
            virtual_env_bin(Some(root.as_os_str())),
            Some(root.join(if cfg!(windows) { "Scripts" } else { "bin" }))
        );
        assert_eq!(virtual_env_bin(None), None);
    }
}
