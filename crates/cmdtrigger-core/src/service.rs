//! The background service: runs `--run <data folder>` at boot as root (Linux, systemd) or as
//! a Windows service, so "background" commands and automations work with nobody logged in.
//! macOS has none: the tray app starts at login instead.

use std::path::PathBuf;

pub const SERVICE_NAME: &str = "cmdtriggeragent";

/// Is there a background service on this platform?
pub fn supported() -> bool {
    cfg!(any(target_os = "linux", windows))
}

fn data_folder(folder: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(f) = folder {
        return Ok(f);
    }
    // Under sudo, use the invoking user's folder, not root's (installdaemon.sh does the same).
    #[cfg(target_os = "linux")]
    if let Ok(user) = std::env::var("SUDO_USER")
        && let Ok(passwd) = std::fs::read_to_string("/etc/passwd")
        && let Some(home) = passwd.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.first() == Some(&user.as_str())).then(|| f.get(5).map(|h| h.to_string())).flatten()
        })
    {
        return Ok(PathBuf::from(home).join(".config").join("cmdtrigger"));
    }
    Ok(crate::DataDir::resolve(None)?.root().to_path_buf())
}

/// The systemd unit for `exe --run <folder>`.
pub fn systemd_unit(exe: &std::path::Path, folder: &std::path::Path) -> String {
    format!(
        "[Unit]\nDescription=CmdTrigger agent (background commands and automations)\nAfter=network-online.target\nWants=network-online.target\n\n\
         [Service]\nType=simple\nExecStart=\"{}\" --run \"{}\"\nUser=root\nRestart=always\nRestartSec=5\n\n\
         [Install]\nWantedBy=multi-user.target\n",
        exe.display(),
        folder.display()
    )
}

#[cfg(target_os = "linux")]
fn run(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    anyhow::ensure!(status.success(), "{program} {} failed ({status})", args.join(" "));
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install(folder: Option<PathBuf>) -> anyhow::Result<String> {
    let folder = data_folder(folder)?;
    let exe = std::env::current_exe()?;
    let unit = format!("/etc/systemd/system/{SERVICE_NAME}.service");
    std::fs::write(&unit, systemd_unit(&exe, &folder))
        .map_err(|e| anyhow::anyhow!("cannot write {unit} ({e}); run this as root"))?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "--now", SERVICE_NAME])?;
    Ok(format!("Installed and started {SERVICE_NAME} for {}.", folder.display()))
}

#[cfg(target_os = "linux")]
pub fn remove() -> anyhow::Result<String> {
    let unit = format!("/etc/systemd/system/{SERVICE_NAME}.service");
    let _ = run("systemctl", &["disable", "--now", SERVICE_NAME]);
    if std::path::Path::new(&unit).exists() {
        std::fs::remove_file(&unit).map_err(|e| anyhow::anyhow!("cannot remove {unit} ({e}); run this as root"))?;
    }
    run("systemctl", &["daemon-reload"])?;
    Ok(format!("Removed {SERVICE_NAME}."))
}

#[cfg(windows)]
pub fn install(folder: Option<PathBuf>) -> anyhow::Result<String> {
    let folder = data_folder(folder)?;
    let exe = std::env::current_exe()?;
    let bin = format!("\"{}\" --service \"{}\"", exe.display(), folder.display());
    let status = std::process::Command::new("sc.exe")
        .args(["create", "CmdTriggerAgent", "start=", "auto", "DisplayName=", "CmdTrigger Agent", "binPath=", &bin])
        .status()?;
    anyhow::ensure!(status.success(), "sc create failed ({status}); run this as an administrator");
    let _ = std::process::Command::new("sc.exe").args(["start", "CmdTriggerAgent"]).status();
    Ok("Installed and started the CmdTriggerAgent service.".into())
}

#[cfg(windows)]
pub fn remove() -> anyhow::Result<String> {
    let _ = std::process::Command::new("sc.exe").args(["stop", "CmdTriggerAgent"]).status();
    let status = std::process::Command::new("sc.exe").args(["delete", "CmdTriggerAgent"]).status()?;
    anyhow::ensure!(status.success(), "sc delete failed ({status}); run this as an administrator");
    Ok("Removed the CmdTriggerAgent service.".into())
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn install(_folder: Option<PathBuf>) -> anyhow::Result<String> {
    let _ = data_folder;
    anyhow::bail!("There is no background service on this platform; the tray app runs foreground commands.")
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn remove() -> anyhow::Result<String> {
    anyhow::bail!("There is no background service on this platform.")
}

/// How the tray app asks for admin rights to run `--install-daemon` / `--remove-daemon`.
pub fn elevated_command(args: &[&str]) -> anyhow::Result<std::process::Command> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    {
        let mut c = std::process::Command::new("pkexec");
        c.arg(exe).args(args);
        Ok(c)
    }
    #[cfg(windows)]
    {
        let list = args.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect::<Vec<_>>().join(",");
        let script = format!("Start-Process -Verb RunAs -Wait -FilePath '{}' -ArgumentList {list}", exe.display());
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", &script]);
        Ok(c)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (exe, args);
        anyhow::bail!("There is no background service on this platform.")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_runs_the_background_agent() {
        let unit = super::systemd_unit("/usr/bin/cmdtrigger".as_ref(), "/home/me/.config/cmdtrigger".as_ref());
        assert!(unit.contains("ExecStart=\"/usr/bin/cmdtrigger\" --run \"/home/me/.config/cmdtrigger\""));
        assert!(unit.contains("User=root"));
    }
}
