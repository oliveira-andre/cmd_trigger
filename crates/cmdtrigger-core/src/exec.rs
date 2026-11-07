//! Turning a run request into a shell command line, and running it.
//!
//! With parameters allowed, "off"/"on" pick `offCommand`/`command` when an off command
//! exists; anything else is appended to the command (quoted when `quoteParams` is set).
//! HomeKit lights add their own twist: brightness becomes the parameter.

use crate::model::Command;
use std::process::Stdio;

/// A command that uses the variable already has its parameters, so they aren't added as well.
/// Matches the name alone, which covers `$X`, `${X}` and Windows's `%X%`.
fn reads_params(line: &str) -> bool {
    line.contains("CMDTRIGGER_PARAMS")
}

pub fn command_line(cmd: &Command, params: Option<&str>) -> String {
    let base = cmd.command().to_string();
    let Some(params) = params.filter(|p| !p.trim().is_empty() && cmd.allow_params()) else { return base };
    let word = params.trim().to_lowercase();
    match cmd.off_command() {
        Some(off) if word == "off" => off.to_string(),
        Some(_) if word == "on" => base,
        _ if reads_params(&base) => base,
        _ if cmd.quote_params() => format!("{base} \"{}\"", params.replace('"', "\\\"")),
        _ => format!("{base} {params}"),
    }
}

/// A HomeKit light: off runs the off command; with parameters allowed, on appends "on" or the
/// brightness. `None` when there is nothing to run.
pub fn homekit_command_line(cmd: &Command, is_on: bool, brightness: Option<u8>) -> Option<String> {
    if cmd.command().trim().is_empty() {
        return None;
    }
    let base = cmd.command();
    Some(match (is_on, cmd.off_command(), brightness, cmd.allow_params()) {
        (false, Some(off), _, _) => off.to_string(),
        _ if reads_params(base) => base.to_string(),
        (_, _, Some(level), true) => format!("{base} {level}"),
        (on, _, _, true) => format!("{base} {}", if on { "on" } else { "off" }),
        _ => base.to_string(),
    })
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub exit_code: Option<i32>,
    /// Set when the process died from a signal (Unix).
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub spawn_error: Option<String>,
}

impl Outcome {
    pub fn success(&self) -> bool {
        self.spawn_error.is_none() && self.exit_code == Some(0)
    }

    /// stdout and stderr together, for the run log.
    pub fn combined(&self) -> String {
        let mut text = self.stdout.trim_end().to_string();
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim_end());
        }
        if let Some(e) = &self.spawn_error {
            text.push_str(&format!("\ncould not start the shell: {e}"));
        }
        if let Some(sig) = self.signal {
            text.push_str(&format!("\nkilled by signal {sig}"));
        }
        text.trim().to_string()
    }
}

fn shell(line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        // Like Node's exec: cmd.exe /d /s /c "<line>", passed verbatim.
        let mut c = tokio::process::Command::new(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()));
        c.raw_arg(format!("/d /s /c \"{line}\""));
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = tokio::process::Command::new("/bin/sh");
        c.arg("-c").arg(line);
        c
    }
}

/// Run a command line through the shell, with extra environment variables, and wait for it.
pub async fn run_shell(line: &str, env: &[(&str, String)]) -> Outcome {
    let mut command = shell(line);
    command
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match command.output().await {
        Ok(output) => output,
        Err(e) => return Outcome { spawn_error: Some(e.to_string()), ..Outcome::default() },
    };
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&output.status);
    #[cfg(not(unix))]
    let signal = None;
    Outcome {
        exit_code: output.status.code(),
        signal,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        spawn_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(v: serde_json::Value) -> Command {
        serde_json::from_value::<Command>(v).unwrap().normalize()
    }

    #[test]
    fn lines() {
        let plain = cmd(json!({"command": "notepad", "allowParams": false}));
        assert_eq!(command_line(&plain, Some("x")), "notepad");
        let params = cmd(json!({"command": "say", "allowParams": true}));
        assert_eq!(command_line(&params, Some("hello world")), "say hello world");
        assert_eq!(command_line(&params, None), "say");
        let quoted = cmd(json!({"command": "say", "allowParams": "true", "quoteParams": true}));
        assert_eq!(command_line(&quoted, Some("say \"hi\"")), "say \"say \\\"hi\\\"\"");
        let toggle = cmd(json!({"command": "light on", "offCommand": "light off", "allowParams": true}));
        assert_eq!(command_line(&toggle, Some(" OFF ")), "light off");
        assert_eq!(command_line(&toggle, Some("on")), "light on");
        assert_eq!(command_line(&toggle, Some("50")), "light on 50");
    }

    #[test]
    fn a_command_using_the_variable_gets_nothing_added() {
        for line in ["say $CMDTRIGGER_PARAMS", "say \"${CMDTRIGGER_PARAMS}\"", "echo %CMDTRIGGER_PARAMS%"] {
            let plain = cmd(json!({"command": line, "allowParams": true}));
            assert_eq!(command_line(&plain, Some("hello")), line);
            let quoted = cmd(json!({"command": line, "allowParams": true, "quoteParams": true}));
            assert_eq!(command_line(&quoted, Some("hello")), line);
        }
        // "off" and "on" still pick the line.
        let toggle =
            cmd(json!({"command": "light $CMDTRIGGER_PARAMS", "offCommand": "light off", "allowParams": true}));
        assert_eq!(command_line(&toggle, Some("off")), "light off");
        assert_eq!(command_line(&toggle, Some("on")), "light $CMDTRIGGER_PARAMS");
        assert_eq!(command_line(&toggle, Some("50")), "light $CMDTRIGGER_PARAMS");
        // Only the command's own line counts, not the parameters.
        let say = cmd(json!({"command": "say", "allowParams": true}));
        assert_eq!(command_line(&say, Some("$CMDTRIGGER_PARAMS")), "say $CMDTRIGGER_PARAMS");
    }

    #[test]
    fn homekit_lines() {
        let plain = cmd(json!({"command": "calc"}));
        assert_eq!(homekit_command_line(&plain, true, None).unwrap(), "calc");
        let toggle = cmd(json!({"command": "lamp", "offCommand": "lamp-off", "allowParams": true}));
        assert_eq!(homekit_command_line(&toggle, false, None).unwrap(), "lamp-off");
        assert_eq!(homekit_command_line(&toggle, true, None).unwrap(), "lamp on");
        assert_eq!(homekit_command_line(&toggle, true, Some(40)).unwrap(), "lamp 40");
        assert!(homekit_command_line(&cmd(json!({"name": "x"})), true, None).is_none());

        let reads = cmd(json!({"command": "lamp $CMDTRIGGER_PARAMS", "offCommand": "lamp-off", "allowParams": true}));
        assert_eq!(homekit_command_line(&reads, true, Some(40)).unwrap(), "lamp $CMDTRIGGER_PARAMS");
        assert_eq!(homekit_command_line(&reads, true, None).unwrap(), "lamp $CMDTRIGGER_PARAMS");
        assert_eq!(homekit_command_line(&reads, false, None).unwrap(), "lamp-off");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn runs_through_cmd_exe() {
        let ok = run_shell("echo %CMDTRIGGER_TEST%", &[("CMDTRIGGER_TEST", "hi there".into())]).await;
        assert!(ok.success(), "{ok:?}");
        assert_eq!(ok.stdout.trim(), "hi there");
        let both = run_shell("echo \"a b\" & echo done", &[]).await;
        assert!(both.stdout.contains("\"a b\"") && both.stdout.contains("done"), "{both:?}");
        let failed = run_shell("exit /b 3", &[]).await;
        assert_eq!(failed.exit_code, Some(3));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_with_env_and_reports_codes() {
        let ok = run_shell("printf %s \"$CMDTRIGGER_TEST\"", &[("CMDTRIGGER_TEST", "hi".into())]).await;
        assert!(ok.success());
        assert_eq!(ok.stdout, "hi");
        let failed = run_shell("echo oops >&2; exit 3", &[]).await;
        assert_eq!(failed.exit_code, Some(3));
        assert_eq!(failed.combined(), "oops");
    }
}
