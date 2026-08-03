use std::process::Command;

use super::ToolOutput;

pub struct ProcessTools;

impl ProcessTools {
    pub fn diagnostic(scope: &str, process_count: usize) -> ToolOutput {
        ToolOutput {
            operation: "process_diagnostic".to_string(),
            message: format!(
                "Process diagnostic for {scope}: observed_process_count={process_count}."
            ),
            claims: vec![format!(
                "CLAIM operation=process_diagnostic observed_process_count={process_count}"
            )],
        }
    }
}

pub fn observe_process_count() -> Result<usize, String> {
    #[cfg(target_os = "linux")]
    {
        let entries = std::fs::read_dir("/proc").map_err(|error| {
            format!("Unable to read the operating-system process table: {error}")
        })?;
        let mut count = 0usize;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("Unable to read an operating-system process entry: {error}")
            })?;
            if entry
                .file_name()
                .to_string_lossy()
                .chars()
                .all(|character| character.is_ascii_digit())
            {
                count += 1;
            }
        }
        require_nonzero_process_count(count)
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid="])
            .output()
            .map_err(|error| {
                format!("Unable to execute the operating-system process probe: {error}")
            })?;
        if !output.status.success() {
            return Err(format!(
                "The operating-system process probe exited with status {}.",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ));
        }
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|error| format!("Process observation was not valid UTF-8: {error}"))?;
        count_process_ids(stdout)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("Process observation is unsupported on this operating system.".to_string())
    }
}

fn count_process_ids(stdout: &str) -> Result<usize, String> {
    let mut count = 0usize;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        line.trim().parse::<u64>().map_err(|_| {
            "Process observation contained a non-numeric process identifier.".to_string()
        })?;
        count += 1;
    }
    require_nonzero_process_count(count)
}

fn require_nonzero_process_count(count: usize) -> Result<usize, String> {
    if count == 0 {
        return Err("Process observation returned no process identifiers.".to_string());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_observation_uses_the_host_process_table() {
        let process_count = observe_process_count().expect("host process table is observable");
        assert!(process_count > 0);
    }

    #[test]
    fn process_id_parser_rejects_empty_or_malformed_observations() {
        assert_eq!(count_process_ids("  1\n42\n 9001 \n").unwrap(), 3);
        assert!(count_process_ids("").is_err());
        assert!(count_process_ids("12\nnot-a-pid\n").is_err());
    }
}
