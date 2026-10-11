# installed by trailboss
# managed by trailboss; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# TRAILBOSS_INTEGRATION_ID=mastracode
# TRAILBOSS_INTEGRATION_VERSION=3

param([string]$Action = "")

if ($Action -notin @("session", "working", "idle", "waiting")) { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

$sessionId = if ($null -ne $payload -and $payload.session_id -is [string]) { $payload.session_id } else { $null }
$seq = [DateTime]::UtcNow.Ticks
$trailboss = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "trailboss" } else { $env:HERDR_BIN_PATH }
try {
    if ($Action -eq "session") {
        if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }
        & $trailboss pane report-agent-session $env:HERDR_PANE_ID --source herdr:mastracode --agent mastracode --seq $seq --session-start-source startup --agent-session-id $sessionId 2>$null | Out-Null
    } else {
        $args = @("pane", "report-agent", $env:HERDR_PANE_ID, "--source", "herdr:mastracode", "--agent", "mastracode", "--state", $Action, "--seq", "$seq")
        if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
            $args += @("--agent-session-id", $sessionId)
        }
        & $trailboss @args 2>$null | Out-Null
    }
} catch {
}
