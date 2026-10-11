# installed by trailboss
# managed by trailboss; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# TRAILBOSS_INTEGRATION_ID=droid
# TRAILBOSS_INTEGRATION_VERSION=3

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

if ($null -eq $payload -or [string]::IsNullOrWhiteSpace($payload.session_id)) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$trailboss = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "trailboss" } else { $env:HERDR_BIN_PATH }
try {
    & $trailboss pane report-agent-session $env:HERDR_PANE_ID --source herdr:droid --agent droid --agent-session-id $payload.session_id --seq $seq 2>$null | Out-Null
} catch {
}
