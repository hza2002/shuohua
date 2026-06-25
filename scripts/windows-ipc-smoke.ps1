param(
    [string]$ExePath = ".\target\x86_64-pc-windows-msvc\debug\shuo.exe",
    [int]$ClientCount = 20,
    [string]$LogDir = "",
    [switch]$StopExisting
)

$ErrorActionPreference = "Stop"

function Resolve-ShuoExe {
    param([string]$Path)
    $resolved = Resolve-Path -LiteralPath $Path -ErrorAction Stop
    return $resolved.Path
}

function Get-IntegrityLabel {
    $groups = whoami /groups
    $line = $groups | Select-String "S-1-16|Mandatory Label|标签" | Select-Object -Last 1
    if ($null -eq $line) {
        return ""
    }
    return $line.ToString().Trim()
}

function Stop-MatchingShuo {
    param([string]$Exe)
    Get-Process shuo -ErrorAction SilentlyContinue | ForEach-Object {
        try {
            if ($_.Path -eq $Exe) {
                Stop-Process -Id $_.Id -Force -ErrorAction Stop
            }
        } catch {
            throw "stop existing shuo process $($_.Id): $_"
        }
    }
}

$exe = Resolve-ShuoExe $ExePath
$root = (Get-Location).Path
if ($LogDir -eq "") {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $LogDir = Join-Path $env:TEMP "shuohua-windows-ipc-smoke-$stamp"
}

New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

if ($StopExisting) {
    Stop-MatchingShuo $exe
    Start-Sleep -Milliseconds 500
}

$daemon = $null
$failures = New-Object System.Collections.Generic.List[string]

try {
    & $exe --version > (Join-Path $LogDir "version.out.txt") 2> (Join-Path $LogDir "version.err.txt")
    $versionCode = $LASTEXITCODE
    if ($versionCode -ne 0) {
        $failures.Add("version exited $versionCode")
    }

    $daemon = Start-Process -FilePath $exe `
        -ArgumentList "--daemon" `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $LogDir "daemon.out.txt") `
        -RedirectStandardError (Join-Path $LogDir "daemon.err.txt") `
        -PassThru `
        -WindowStyle Hidden
    Start-Sleep -Seconds 2

    if ($daemon.HasExited) {
        $failures.Add("daemon exited early with $($daemon.ExitCode)")
    }

    & $exe service status > (Join-Path $LogDir "status.out.txt") 2> (Join-Path $LogDir "status.err.txt")
    $statusCode = $LASTEXITCODE
    if ($statusCode -ne 0) {
        $failures.Add("service status exited $statusCode")
    }

    & $exe --daemon > (Join-Path $LogDir "second-daemon.out.txt") 2> (Join-Path $LogDir "second-daemon.err.txt")
    $secondCode = $LASTEXITCODE
    if ($secondCode -eq 0) {
        $failures.Add("second daemon started successfully")
    }

    $busyDir = Join-Path $LogDir "busy"
    New-Item -ItemType Directory -Path $busyDir -Force | Out-Null
    $jobs = 1..$ClientCount | ForEach-Object {
        $index = $_
        Start-Job -ScriptBlock {
            param($Exe, $Root, $BusyDir, $Index)
            Set-Location $Root
            & $Exe service status > (Join-Path $BusyDir "client-$Index.out.txt") 2> (Join-Path $BusyDir "client-$Index.err.txt")
            Set-Content -Path (Join-Path $BusyDir "client-$Index.exit.txt") -Value $LASTEXITCODE -Encoding ASCII
            exit $LASTEXITCODE
        } -ArgumentList $exe, $root, $busyDir, $index
    }
    $jobs | Wait-Job | Out-Null
    $jobs | Receive-Job -ErrorAction SilentlyContinue | Out-Null
    $jobs | Remove-Job

    $busyCodes = Get-ChildItem $busyDir -Filter "client-*.exit.txt" |
        ForEach-Object { [int](Get-Content $_.FullName -Raw) }
    $busyExit0 = ($busyCodes | Where-Object { $_ -eq 0 }).Count
    $busyNonzero = @($busyCodes | Where-Object { $_ -ne 0 })
    if ($busyCodes.Count -ne $ClientCount) {
        $failures.Add("busy smoke wrote $($busyCodes.Count) exit files, expected $ClientCount")
    }
    if ($busyNonzero.Count -gt 0) {
        $failures.Add("busy smoke had nonzero exits: $($busyNonzero -join ',')")
    }

    & $exe service status > (Join-Path $LogDir "after-busy-status.out.txt") 2> (Join-Path $LogDir "after-busy-status.err.txt")
    $afterBusyStatusCode = $LASTEXITCODE
    if ($afterBusyStatusCode -ne 0) {
        $failures.Add("after-busy service status exited $afterBusyStatusCode")
    }

    $daemonRunning = $false
    if ($null -ne $daemon) {
        $daemonRunning = -not $daemon.HasExited
        if (-not $daemonRunning) {
            $failures.Add("daemon was not running after smoke")
        }
    }

    $summary = [ordered]@{
        exe = $exe
        log = $LogDir
        integrity = Get-IntegrityLabel
        version_exit = $versionCode
        daemon_pid = if ($null -ne $daemon) { $daemon.Id } else { $null }
        daemon_running_before_stop = $daemonRunning
        status_exit = $statusCode
        second_daemon_exit = $secondCode
        busy_total = $ClientCount
        busy_exit_files = $busyCodes.Count
        busy_exit_0 = $busyExit0
        busy_nonzero = $busyNonzero
        after_busy_status_exit = $afterBusyStatusCode
        status_out = Get-Content (Join-Path $LogDir "status.out.txt") -Raw -ErrorAction SilentlyContinue
        second_daemon_err = Get-Content (Join-Path $LogDir "second-daemon.err.txt") -Raw -ErrorAction SilentlyContinue
        after_busy_status_out = Get-Content (Join-Path $LogDir "after-busy-status.out.txt") -Raw -ErrorAction SilentlyContinue
        failures = @($failures)
    }

    $summary | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $LogDir "summary.json") -Encoding UTF8
    $summary | ConvertTo-Json -Depth 5

    if ($failures.Count -gt 0) {
        exit 1
    }
} finally {
    if ($null -ne $daemon -and -not $daemon.HasExited) {
        Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue
    }
}
