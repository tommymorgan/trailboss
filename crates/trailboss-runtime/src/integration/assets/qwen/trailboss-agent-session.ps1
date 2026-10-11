# installed by trailboss
# managed by trailboss; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# TRAILBOSS_INTEGRATION_ID=qwen
# TRAILBOSS_INTEGRATION_VERSION=1

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_SOCKET_PATH)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

if ($null -eq $payload -or [string]::IsNullOrWhiteSpace($payload.session_id)) { exit 0 }

$seq = [DateTime]::UtcNow.Ticks
$trailboss = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "trailboss" } else { $env:HERDR_BIN_PATH }
$commandArgs = @(
    "pane", "report-agent-session", $env:HERDR_PANE_ID,
    "--source", "herdr:qwen", "--agent", "qwen",
    "--agent-session-id", [string]$payload.session_id,
    "--seq", [string]$seq
)
if ($payload.source -in @("startup", "resume", "clear", "compact", "branch")) {
    $commandArgs += @("--session-start-source", [string]$payload.source)
}
try {
    & $trailboss @commandArgs 2>$null | Out-Null
} catch {
}
