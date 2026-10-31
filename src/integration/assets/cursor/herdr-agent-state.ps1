# managed by herdr; reinstalling the integration replaces this file.
# HERDR_INTEGRATION_ID=cursor
# HERDR_INTEGRATION_VERSION=3

param([string]$Action = "")

function Exit-Hook {
    Write-Output "{}"
    exit 0
}

if ($Action -ne "session") { Exit-Hook }
if ($env:HERDR_ENV -ne "1") { Exit-Hook }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { Exit-Hook }

# Read hook input as raw bytes so UTF-8 payloads decode correctly even when the
# console input code page is not UTF-8 (for example CP949 on Korean Windows).
$stdinReader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), [System.Text.Encoding]::UTF8)
$inputText = $stdinReader.ReadToEnd()
$jsonStart = $inputText.IndexOf("{")
if ($jsonStart -gt 0) {
    $inputText = $inputText.Substring($jsonStart)
}
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    Exit-Hook
}

if ($null -eq $payload) { Exit-Hook }
$event = if ($payload.hook_event_name -is [string]) { $payload.hook_event_name } else { $payload.hookEventName }
if (-not [string]::IsNullOrWhiteSpace($event) -and $event -ne "sessionStart") { Exit-Hook }

$sessionId = $null
# The conversation id names the chat and its transcript; session_id can be the CLI session.
foreach ($name in @("conversation_id", "conversationId", "session_id", "sessionId")) {
    $value = $payload.$name
    if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) {
        $sessionId = $value
        break
    }
}
if ([string]::IsNullOrWhiteSpace($sessionId)) { Exit-Hook }

$transcriptPath = $null
foreach ($name in @("transcript_path", "transcriptPath")) {
    $value = $payload.$name
    if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) {
        $transcriptPath = $value
        break
    }
}

$seq = [DateTime]::UtcNow.Ticks
$herdr = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "herdr" } else { $env:HERDR_BIN_PATH }
try {
    $args = @("pane", "report-agent-session", $env:HERDR_PANE_ID, "--source", "herdr:cursor", "--agent", "cursor", "--seq", "$seq", "--agent-session-id", "$sessionId")
    if ($transcriptPath) { $args += @("--agent-session-path", "$transcriptPath") }
    & $herdr @args 2>$null | Out-Null
} catch {
}

Exit-Hook
