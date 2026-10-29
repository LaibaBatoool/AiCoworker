//! Step-up authentication with Windows Hello (fingerprint, face or PIN).
//!
//! Problem (supervisor / committee question): the Approve button proves that
//! SOMEONE is at the PC, not that it is the account owner. A colleague at an
//! unlocked laptop could approve "send this email" or "delete that folder".
//!
//! What this does: when switched on, every PRIVILEGED action, after the user
//! clicks Approve, must also pass a Windows Hello check before it runs.
//! Safe and mutating actions are not affected (mutating ones are undoable),
//! so normal work stays fast. This is "step-up" authentication: ask for
//! identity only when the risk is high.
//!
//! Safety properties:
//!  - enforced inside permission::checkpoint(), so neither the AI nor the
//!    frontend can skip it (same as the approval itself);
//!  - turning it ON and turning it OFF both need a successful Hello check,
//!    so whoever is at the PC can't simply switch it off;
//!  - Windows does the biometric check; the app never sees a fingerprint,
//!    face or PIN, only "verified" / "not verified";
//!  - every check (verified, cancelled, failed) goes to
//!    %APPDATA%\AiCoworker\hello-log.jsonl;
//!  - the setting lives in %APPDATA%\AiCoworker\settings.json;
//!    AICOWORKER_HELLO=off (set by the benchmark) disables it for headless runs.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = not loaded yet, 1 = off, 2 = on
static REQUIRED: AtomicU8 = AtomicU8::new(0);
#[cfg_attr(test, allow(dead_code))]
const HELLO_TIMEOUT_SECS: u64 = 90;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct HelloStatus {
    pub required: bool,
    /// "Available", "DeviceNotPresent", "NotConfiguredForUser", "DisabledByPolicy", "DeviceBusy", or an error
    pub availability: String,
}

fn app_dir() -> PathBuf {
    let base = std::env::var("APPDATA").or_else(|_| std::env::var("XDG_CONFIG_HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("AiCoworker")
}

fn settings_path() -> PathBuf {
    app_dir().join("settings.json")
}

fn env_off() -> bool {
    std::env::var("AICOWORKER_HELLO").map(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "false")).unwrap_or(false)
}

/// Is Windows Hello required for privileged actions right now?
pub fn required() -> bool {
    if env_off() {
        return false;
    }
    match REQUIRED.load(Ordering::Relaxed) {
        2 => true,
        1 => false,
        _ => {
            let on = load_setting();
            REQUIRED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

#[cfg(not(test))]
fn load_setting() -> bool {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("require_windows_hello").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

/// Tests never read the real settings file (a developer who switched Hello on
/// would otherwise get fingerprint prompts during `cargo test`).
#[cfg(test)]
fn load_setting() -> bool {
    false
}

fn save_setting(on: bool) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let p = settings_path();
    let mut v: serde_json::Value = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !v.is_object() {
        v = serde_json::json!({});
    }
    v["require_windows_hello"] = serde_json::Value::Bool(on);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Could not save the setting: {}", e))?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| format!("Could not save the setting: {}", e))
}

fn log(action: &str, result: &str) {
    if cfg!(test) {
        return;
    }
    let line = serde_json::json!({
        "timestamp_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "action": action,
        "result": result,
    });
    let p = app_dir().join("hello-log.jsonl");
    let _ = std::fs::create_dir_all(app_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        use std::io::Write;
        let _ = writeln!(f, "{}", line);
    }
}

// ---------------------------------------------------------------
// Windows Hello through WinRT (UserConsentVerifier), via a fixed PowerShell
// script (same runner as the desktop tools: the message travels in an env var).
// ---------------------------------------------------------------

#[cfg_attr(test, allow(dead_code))]
const HELLO_PS: &str = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Security.Credentials.UI.UserConsentVerifier, Windows.Security.Credentials.UI, ContentType=WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
function Wait-WinRt($op, [Type]$type) { $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); [void]$t.Wait(-1); $t.Result }
$avail = Wait-WinRt ([Windows.Security.Credentials.UI.UserConsentVerifier]::CheckAvailabilityAsync()) ([Windows.Security.Credentials.UI.UserConsentVerifierAvailability])
if ($env:AICW_CHECK_ONLY -eq '1' -or "$avail" -ne 'Available') {
  [pscustomobject]@{ availability = "$avail"; result = '' } | ConvertTo-Json -Compress
  exit 0
}
$r = Wait-WinRt ([Windows.Security.Credentials.UI.UserConsentVerifier]::RequestVerificationAsync($env:AICW_MSG)) ([Windows.Security.Credentials.UI.UserConsentVerificationResult])
[pscustomobject]@{ availability = "$avail"; result = "$r" } | ConvertTo-Json -Compress
"#;

/// Returns (availability, result). result is "" when only availability was asked.
#[cfg(not(test))]
fn platform(message: &str, check_only: bool) -> Result<(String, String), String> {
    let v = crate::tools::desktop::run_ps_json_for(
        HELLO_PS,
        &[("AICW_MSG", message.to_string()), ("AICW_CHECK_ONLY", if check_only { "1" } else { "0" }.to_string())],
        HELLO_TIMEOUT_SECS,
    )?;
    Ok((v["availability"].as_str().unwrap_or("").to_string(), v["result"].as_str().unwrap_or("").to_string()))
}

#[cfg(test)]
thread_local! {
    static FAKE: std::cell::RefCell<(String, String, usize)> = std::cell::RefCell::new(("Available".into(), "Verified".into(), 0));
}

#[cfg(test)]
fn platform(_message: &str, check_only: bool) -> Result<(String, String), String> {
    FAKE.with(|f| {
        let mut f = f.borrow_mut();
        if !check_only {
            f.2 += 1;
        }
        Ok((f.0.clone(), if check_only { String::new() } else { f.1.clone() }))
    })
}

/// Pure: turns Windows' answer into Ok / a clear error.
pub fn interpret(availability: &str, result: &str) -> Result<(), String> {
    match (availability, result) {
        ("Available", "Verified") => Ok(()),
        ("Available", "Canceled") => Err("Windows Hello was cancelled".into()),
        ("Available", "RetriesExhausted") => Err("Windows Hello failed too many times".into()),
        ("Available", other) => Err(format!("Windows Hello did not verify you ({})", if other.is_empty() { "no answer" } else { other })),
        ("NotConfiguredForUser", _) => Err("Windows Hello is not set up for this Windows account (Settings > Accounts > Sign-in options)".into()),
        ("DeviceNotPresent", _) => Err("this PC has no Windows Hello device (fingerprint reader, camera or PIN)".into()),
        ("DisabledByPolicy", _) => Err("Windows Hello is turned off by a system policy".into()),
        (other, _) => Err(format!("Windows Hello is not available ({})", if other.is_empty() { "unknown" } else { other })),
    }
}

/// Ask Windows Hello right now. `reason` is shown in the Windows prompt.
pub fn verify(reason: &str) -> Result<(), String> {
    let message = format!("AI CoWorker: {}", reason.chars().take(200).collect::<String>());
    let outcome = platform(&message, false).and_then(|(a, r)| interpret(&a, &r));
    log(reason, match &outcome {
        Ok(()) => "verified",
        Err(e) => e.as_str(),
    });
    outcome
}

/// Called by permission::checkpoint for a CONFIRMED privileged action.
pub fn step_up(action: &str) -> Result<(), String> {
    if !required() {
        return Ok(());
    }
    verify(&format!("confirm it's you: {}", action)).map_err(|e| {
        format!(
            "USER_VERIFICATION_FAILED: {}. The action was NOT run. Do not retry it unless the user asks again.",
            e
        )
    })
}

pub fn status() -> HelloStatus {
    HelloStatus {
        required: required(),
        availability: match platform("", true) {
            Ok((a, _)) => a,
            Err(e) => e,
        },
    }
}

/// Switching it on OR off needs a successful Hello check first.
pub fn set_required(on: bool) -> Result<HelloStatus, String> {
    if env_off() {
        return Err("Windows Hello is disabled by AICOWORKER_HELLO=off for this session.".into());
    }
    verify(if on { "turn on Windows Hello for risky actions" } else { "turn off Windows Hello for risky actions" })?;
    save_setting(on)?;
    REQUIRED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    Ok(status())
}

// ---------------------------------------------------------------
// Tests: `cargo test auth`  (fake Windows Hello; no prompts)
// ---------------------------------------------------------------
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{checkpoint, PermissionTier};

    fn fake(avail: &str, result: &str) {
        FAKE.with(|f| *f.borrow_mut() = (avail.into(), result.into(), 0));
    }
    fn prompts() -> usize {
        FAKE.with(|f| f.borrow().2)
    }

    #[test]
    fn answers_are_interpreted() {
        assert!(interpret("Available", "Verified").is_ok());
        assert!(interpret("Available", "Canceled").unwrap_err().contains("cancelled"));
        assert!(interpret("NotConfiguredForUser", "").unwrap_err().contains("not set up"));
        assert!(interpret("DeviceNotPresent", "").is_err());
        assert!(interpret("Available", "").is_err(), "no answer is never a yes");
        assert!(interpret("", "Verified").is_err(), "fail closed when availability is unknown");
    }

    #[test]
    fn off_by_default_and_never_prompts() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        REQUIRED.store(1, Ordering::Relaxed);
        fake("Available", "Verified");
        assert!(checkpoint(&PermissionTier::Privileged, "delete_file: a.txt", true).is_ok());
        assert_eq!(prompts(), 0);
    }

    #[test]
    fn when_on_only_confirmed_privileged_actions_prompt() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fake("Available", "Verified");
        REQUIRED.store(2, Ordering::Relaxed);
        assert!(checkpoint(&PermissionTier::Safe, "read_file", true).is_ok());
        assert!(checkpoint(&PermissionTier::Mutating, "write_file", true).is_ok());
        let e = checkpoint(&PermissionTier::Privileged, "delete_file: a.txt", false).unwrap_err();
        assert!(e.starts_with("PRIVILEGED_CONFIRMATION_REQUIRED"), "Approve still comes first");
        assert_eq!(prompts(), 0, "no Hello prompt for safe, mutating or not-yet-approved");
        assert!(checkpoint(&PermissionTier::Privileged, "delete_file: a.txt", true).is_ok());
        assert_eq!(prompts(), 1);
        REQUIRED.store(1, Ordering::Relaxed);
    }

    #[test]
    fn a_failed_or_cancelled_check_blocks_the_action() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        REQUIRED.store(2, Ordering::Relaxed);
        for (a, r) in [("Available", "Canceled"), ("Available", "RetriesExhausted"), ("NotConfiguredForUser", "")] {
            fake(a, r);
            let e = checkpoint(&PermissionTier::Privileged, "send_email", true).unwrap_err();
            assert!(e.starts_with("USER_VERIFICATION_FAILED") && e.contains("NOT run"), "{}", e);
        }
        REQUIRED.store(1, Ordering::Relaxed);
    }

    #[test]
    fn switching_on_or_off_needs_hello() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        REQUIRED.store(1, Ordering::Relaxed);
        fake("Available", "Canceled");
        assert!(set_required(true).is_err());
        assert!(!required(), "stays off when the check fails");
        fake("Available", "Verified");
        assert!(set_required(true).unwrap().required);
        fake("Available", "Canceled");
        assert!(set_required(false).is_err());
        assert!(required(), "someone at the PC can't just switch it off");
        fake("Available", "Verified");
        assert!(!set_required(false).unwrap().required);
    }

    #[test]
    fn env_switch_disables_it_for_the_benchmark() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        REQUIRED.store(2, Ordering::Relaxed);
        std::env::set_var("AICOWORKER_HELLO", "off");
        fake("Available", "Canceled");
        assert!(checkpoint(&PermissionTier::Privileged, "delete_file", true).is_ok());
        std::env::remove_var("AICOWORKER_HELLO");
        REQUIRED.store(1, Ordering::Relaxed);
    }
}
