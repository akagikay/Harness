use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

pub struct BashPlugin;

/// Lower/upper bounds applied to the caller-supplied `timeout_secs` so a hostile
/// or malformed argument can neither disable the timeout nor stall a session.
pub(crate) const MIN_TIMEOUT_SECS: u64 = 1;
pub(crate) const MAX_TIMEOUT_SECS: u64 = 600;
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Hard per-stream byte budget for captured stdout/stderr. A chatty command
/// therefore cannot grow unbounded harness memory.
const MAX_STREAM_BYTES: usize = 1_000_000;

/// Clamp an optional `timeout_secs` argument into the supported range.
pub(crate) fn clamp_timeout_secs(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS)
}

/// Read at most `cap` bytes from `reader`, reporting whether more bytes remained.
async fn read_capped<R>(reader: R, cap: usize) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    // Read one extra byte so we can tell "exactly at the cap" from "truncated".
    let mut limited = reader.take(cap as u64 + 1);
    if limited.read_to_end(&mut buf).await.is_err() {
        return (buf, false);
    }
    if buf.len() > cap {
        buf.truncate(cap);
        (buf, true)
    } else {
        (buf, false)
    }
}

/// Decode a byte-capped stream, appending a visible notice when truncated.
fn decode_capped(bytes: &[u8], truncated: bool, cap: usize, label: &str) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if truncated {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("[truncated: {} exceeded {} bytes]", label, cap));
    }
    text
}

/// Spawn the shell in `sandbox`, capture bounded stdout/stderr, and honour the
/// timeout.
///
/// HONEST SANDBOX: `current_dir(sandbox)` only sets the *working directory*.
/// The spawned shell inherits the harness process's privileges and is NOT
/// contained: it can `cd` to and read/write any absolute path on the machine,
/// and reach the network. That is why the tool is declared
/// `ShellOrNetwork` and requires approval in Safe mode, and why every
/// invocation is logged (see `BashPlugin::execute`).
async fn execute_shell(
    command_str: &str,
    timeout_secs: u64,
    sandbox: &Path,
) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("powershell");
        c.arg("-NoProfile").arg("-NonInteractive").arg("-Command").arg(command_str);
        c
    };

    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command_str);
        c
    };

    cmd.current_dir(sandbox)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Never leave an orphan/zombie shell behind if the future is dropped
        // (cancellation, session teardown, or a timeout bail-out below).
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("Failed to spawn process: {}", e)),
    };

    // Detach the pipes from the child so the reads do not borrow it: this
    // keeps the child killable (and reapable) once the timeout expires.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // Read both pipes concurrently under a hard byte budget; sequential
    // reads could deadlock when the child fills the unread pipe.
    let capture = async {
        let stdout_fut = async {
            let (bytes, truncated) = match stdout_pipe {
                Some(pipe) => read_capped(pipe, MAX_STREAM_BYTES).await,
                None => (Vec::new(), false),
            };
            decode_capped(&bytes, truncated, MAX_STREAM_BYTES, "stdout")
        };
        let stderr_fut = async {
            let (bytes, truncated) = match stderr_pipe {
                Some(pipe) => read_capped(pipe, MAX_STREAM_BYTES).await,
                None => (Vec::new(), false),
            };
            decode_capped(&bytes, truncated, MAX_STREAM_BYTES, "stderr")
        };
        tokio::join!(stdout_fut, stderr_fut)
    };

    let (stdout, stderr) = match timeout(Duration::from_secs(timeout_secs), capture).await {
        Ok(pair) => pair,
        Err(_) => {
            let _ = child.kill().await;
            // Reap so the killed shell does not linger as a zombie.
            let _ = child.wait().await;
            return Err(format!("Command timed out after {} seconds.", timeout_secs));
        }
    };

    // Bound the reap as well: a command that closed its pipes but never exits
    // must not pin the tool (the outer harness timeout would only cancel us).
    let status = match timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(_)) => None,
        Err(_) => {
            let _ = child.kill().await;
            child.wait().await.ok()
        }
    };
    let code = status.as_ref().and_then(|s| s.code()).unwrap_or(-1);
    let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

    if success {
        let mut res = String::new();
        if !stdout.is_empty() {
            res.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !res.is_empty() { res.push_str("\n--- stderr ---\n"); }
            res.push_str(&stderr);
        }
        if res.is_empty() {
            res.push_str("(Command completed with exit code 0 and no output)");
        }
        Ok(res)
    } else {
        Err(format!(
            "Command failed with exit code {}\nStdout:\n{}\nStderr:\n{}",
            code, stdout, stderr
        ))
    }
}

#[async_trait]
impl ToolPlugin for BashPlugin {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command (PowerShell on Windows, sh on Unix) with the harness process's \
         full privileges. It starts in the sandbox workspace, but the command it runs is NOT \
         contained: any command may `cd`, read and write absolute paths outside the workspace \
         (e.g. `cd C:\\Users`), and reach the network. This tool is declared `ShellOrNetwork` and \
         requires approval in Safe mode; treat every invocation as running code with the user's \
         own rights. Use `read_file`, `list_dir` and `write_file` for anything that must stay \
         inside the sandbox."
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        let command_str = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'command'".to_string())?;

        let timeout_secs = clamp_timeout_secs(args.get("timeout_secs").and_then(|v| v.as_u64()));

        // The shell runs privileged and uncontained. Never pretend otherwise:
        // log every invocation so an operator can audit what ran with what
        // rights, instead of relying on a sandbox that does not exist here.
        eprintln!(
            "[harness-bash] privileged shell (NOT sandboxed) cwd={} timeout={}s cmd={:?}",
            sandbox.display(),
            timeout_secs,
            command_str
        );

        execute_shell(command_str, timeout_secs, sandbox).await
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "shell".to_string(),
            permission_level: ToolPermission::ShellOrNetwork,
            // Advertise the boundary in the field help too: the agent reads this
            // right next to `command` when it decides how to touch a file.
            fields: vec![
                ToolFieldSpec::required_string(
                    "command",
                    "The command line string to run (runs with the harness process's full \
                     privileges, NOT confined to the sandbox workspace).",
                    "Provide a non-empty shell command (PowerShell on Windows, sh on Unix).",
                )
                .non_empty(),
                ToolFieldSpec::optional_int(
                    "timeout_secs",
                    "Optional maximum seconds to allow the command to run (default: 30).",
                )
                .with_min(1.0)
                .with_default(json!(30))
                .with_error_hint("Provide a positive integer >= 1 for timeout_secs."),
            ],
            expected_output: OutputContract::new(
                "text",
                "Combined stdout (and stderr if present) from the executed command, or a zero-exit completion notice.",
                "test result: ok. 7 passed; 0 failed",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "non_zero_exit",
                    "Command exits with a non-zero exit code",
                    "Command failed with exit code <N>\\nStdout:...\\nStderr:...",
                    "Inspect stdout and stderr in the error output, fix the command or code, and retry.",
                ),
                ToolFailureMode::new(
                    "timeout",
                    "Command exceeds timeout_secs",
                    "Command timed out after <N> seconds.",
                    "Increase timeout_secs or avoid long-running interactive commands.",
                ),
                ToolFailureMode::new(
                    "spawn_failed",
                    "Shell process cannot be spawned in the sandbox directory",
                    "Failed to spawn process: <os error>",
                    "Ensure the workspace sandbox directory exists and the system shell is accessible.",
                ),
            ],
            timeout_secs: 30,
            is_custom: false,
        }
    }
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    /// `bash` stays privileged, so its own contract must say so. The old
    /// description claimed "inside the sandbox workspace" while `cd C:\Users`
    /// worked — the tool must never promise containment it does not deliver.
    #[test]
    fn bash_contract_states_that_it_is_not_contained() {
        let plugin = BashPlugin;
        let desc = plugin.description();
        assert!(
            desc.contains("NOT contained") && desc.contains("full privileges"),
            "the description must admit it is not sandboxed: {desc}"
        );
        let contract = plugin.contract();
        assert_eq!(contract.permission_level, ToolPermission::ShellOrNetwork);
        assert!(
            contract.description.contains("NOT contained"),
            "the contract must carry the honest description too"
        );
        let command_help = contract
            .fields
            .iter()
            .find(|f| f.name == "command")
            .expect("command field");
        assert!(
            command_help.description.contains("NOT confined"),
            "the field help must warn the agent: {}",
            command_help.description
        );
    }

    /// The tool must not claim containment in the persisted contract JSON either.
    #[tokio::test]
    async fn executed_shell_runs_outside_the_sandbox_when_told_to() {
        #[cfg(target_os = "windows")]
        let cmd = "cd C:\\; Get-Location | Select-Object -ExpandProperty Path";
        #[cfg(not(target_os = "windows"))]
        let cmd = "cd /etc; pwd";

        let sandbox = std::env::temp_dir().join(format!(
            "harness_bash_escape_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&sandbox).unwrap();
        let plugin = BashPlugin;
        let out = plugin
            .execute(json!({ "command": cmd, "timeout_secs": 20 }), &sandbox)
            .await
            .expect("the command should succeed");
        // The shell escaped the working directory: the tool is privileged, not
        // sandboxed, and must never be documented as if it were.
        let escaped = out.contains("C:\\") || out.contains("/etc");
        assert!(
            escaped,
            "`bash` is privileged and uncontained (cwd={}); output was: {:?}",
            sandbox.display(),
            out
        );
        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn timeout_is_clamped_to_supported_range() {
        assert_eq!(clamp_timeout_secs(None), 30, "omitted timeout defaults to 30s");
        assert_eq!(clamp_timeout_secs(Some(1)), 1, "1s stays 1s");
        assert_eq!(clamp_timeout_secs(Some(45)), 45);
        assert_eq!(clamp_timeout_secs(Some(600)), 600, "600s stays 600s");
        assert_eq!(clamp_timeout_secs(Some(0)), 1, "zero timeout clamps up to 1s");
        assert_eq!(clamp_timeout_secs(Some(u64::MAX)), 600, "huge timeout clamps down to 600s");
    }

    #[test]
    fn truncated_streams_report_visible_notice() {
        let plain = decode_capped(b"hello world", false, 1_000_000, "stdout");
        assert_eq!(plain, "hello world");

        // read_capped already trimmed the payload to `cap` bytes before handing it over.
        let capped = decode_capped(b"abcd", true, 4, "stdout");
        assert_eq!(capped, "abcd\n[truncated: stdout exceeded 4 bytes]");

        let cut_mid_char = decode_capped(&[b'a', 0xF0, 0x9F], true, 2, "stderr");
        assert!(
            cut_mid_char.contains('\u{FFFD}'),
            "lossy decode must substitute invalid UTF-8 instead of panicking: {:?}",
            cut_mid_char
        );
        assert!(cut_mid_char.ends_with("stderr exceeded 2 bytes]"));
    }

    #[tokio::test]
    async fn read_capped_never_exceeds_budget() {
        let payload = vec![b'x'; 1024];
        let (bytes, truncated) = read_capped(payload.as_slice(), 10).await;
        assert_eq!(bytes.len(), 10);
        assert!(truncated);

        let (bytes, truncated) = read_capped(payload.as_slice(), 2048).await;
        assert_eq!(bytes.len(), 1024);
        assert!(!truncated);

        let (bytes, truncated) = read_capped(b"" as &[u8], 10).await;
        assert!(bytes.is_empty());
        assert!(!truncated);
    }

    #[tokio::test]
    async fn spawned_shell_is_killed_and_timeout_is_clamped() {
        let plugin = BashPlugin;
        let sandbox = std::env::temp_dir().join(format!(
            "harness_bash_timeout_{}_{}",
            std::process::id(),
            line!()
        ));
        tokio::fs::create_dir_all(&sandbox).await.unwrap();

        // A command that outlives its clamped 1s budget must be killed, and the
        // tool must report the clamped timeout rather than the raw argument.
        let err = plugin
            .execute(
                json!({
                    "command": "Start-Sleep -Seconds 30",
                    "timeout_secs": 0
                }),
                &sandbox,
            )
            .await
            .expect_err("command exceeding the clamped timeout must fail");
        assert_eq!(err, "Command timed out after 1 seconds.");

        let _ = tokio::fs::remove_dir_all(&sandbox);
    }

    #[tokio::test]
    async fn successful_command_reports_output_and_exit_status() {
        let plugin = BashPlugin;
        let sandbox = std::env::temp_dir().join(format!(
            "harness_bash_ok_{}_{}",
            std::process::id(),
            line!()
        ));
        tokio::fs::create_dir_all(&sandbox).await.unwrap();

        #[cfg(target_os = "windows")]
        let (ok_cmd, fail_cmd) = (
            "echo harness-bash-ok",
            "Write-Error 'boom'; exit 3",
        );
        #[cfg(not(target_os = "windows"))]
        let (ok_cmd, fail_cmd) = (
            "printf 'harness-bash-ok'",
            "echo boom >&2; exit 3",
        );

        let out = plugin
            .execute(json!({ "command": ok_cmd }), &sandbox)
            .await
            .expect("simple echo should succeed");
        assert!(out.contains("harness-bash-ok"), "unexpected output: {}", out);

        let err = plugin
            .execute(json!({ "command": fail_cmd }), &sandbox)
            .await
            .expect_err("non-zero exit must fail");
        assert!(err.contains("Command failed with exit code"), "unexpected: {}", err);
        assert!(err.contains("Stdout:"), "stdout must be surfaced: {}", err);
        assert!(err.contains("Stderr:"), "stderr must be surfaced: {}", err);

        let _ = tokio::fs::remove_dir_all(&sandbox);
    }
}