//! System notifications for the automation "notify" step.

/// Show a notification; best effort (logs when there is no way to show one).
pub async fn show(title: &str, body: &str) {
    let result = platform(title, body).await;
    if let Err(e) = result {
        tracing::info!("Notification ({e}): {title} — {body}");
    }
}

#[cfg(target_os = "macos")]
async fn platform(title: &str, body: &str) -> anyhow::Result<()> {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", quote(body), quote(title));
    let status = tokio::process::Command::new("osascript").arg("-e").arg(script).status().await?;
    anyhow::ensure!(status.success(), "osascript failed");
    Ok(())
}

#[cfg(target_os = "linux")]
async fn platform(title: &str, body: &str) -> anyhow::Result<()> {
    let status =
        tokio::process::Command::new("notify-send").args(["--app-name=CmdTrigger", title, body]).status().await?;
    anyhow::ensure!(status.success(), "notify-send failed");
    Ok(())
}

#[cfg(windows)]
async fn platform(title: &str, body: &str) -> anyhow::Result<()> {
    // A balloon tip from PowerShell: no extra modules needed.
    let quote = |s: &str| s.replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; \
         $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; \
         $n.ShowBalloonTip(5000, '{}', '{}', 'Info'); Start-Sleep -Seconds 6; $n.Dispose()",
        quote(title),
        quote(body)
    );
    tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .spawn()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
async fn platform(_title: &str, _body: &str) -> anyhow::Result<()> {
    anyhow::bail!("no notifications on this platform")
}
