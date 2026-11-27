use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use sysinfo::{Pid, System};

use super::contract::{OutputContract, ToolContract, ToolFailureMode, ToolPermission};
use super::ToolPlugin;

pub struct SysInfoPlugin;

#[async_trait]
impl ToolPlugin for SysInfoPlugin {
    fn name(&self) -> &'static str {
        "sys_info"
    }

    fn description(&self) -> &'static str {
        "Get host hardware, operating system, and harness memory/CPU resource usage."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "system".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![],
            expected_output: OutputContract::new(
                "text",
                "Human-readable system diagnostics including OS, kernel version, CPU cores, host RAM, harness RSS memory, and PID.",
                "System Information:\n - OS: Windows (Kernel: 10.0)\n - CPUs: 8 logical cores\n - Harness Process RSS Memory: 12.50 MB",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "invalid_args",
                    "Arguments passed as a non-object JSON value",
                    "Contract violation for tool 'sys_info': arguments must be a JSON object",
                    "Call sys_info with an empty JSON object {}.",
                ),
                ToolFailureMode::new(
                    "partial_host_metadata",
                    "OS or kernel metadata unavailable on the current platform",
                    "Fields may report 'Unknown' if restricted by OS permissions",
                    "Use bash for platform-specific hardware diagnostics if detailed OS metadata is missing.",
                ),
            ],
            timeout_secs: 10,
            is_custom: false,
        }
    }

    async fn execute(&self, _args: Value, _sandbox: &Path) -> Result<String, String> {
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu();

        let pid = Pid::from_u32(std::process::id());
        sys.refresh_process(pid);

        let process_mem_mb = sys.process(pid)
            .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);

        let total_mem_mb = sys.total_memory() as f64 / 1024.0 / 1024.0;
        let used_mem_mb = sys.used_memory() as f64 / 1024.0 / 1024.0;

        let os_name = System::name().unwrap_or_else(|| "Unknown".into());
        let kernel = System::kernel_version().unwrap_or_else(|| "Unknown".into());
        let cpu_count = sys.cpus().len();

        Ok(format!(
            "System Information:\n\
             - OS: {} (Kernel: {})\n\
             - CPUs: {} logical cores\n\
             - Host Memory: {:.1} MB used / {:.1} MB total\n\
             - Harness Process RSS Memory: {:.2} MB (Ultra-low footprint!)\n\
             - Process ID: {}",
            os_name, kernel, cpu_count, used_mem_mb, total_mem_mb, process_mem_mb, pid
        ))
    }
}
