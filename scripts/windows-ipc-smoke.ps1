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

function Read-DaemonPid {
    param([string]$Text)
    if ($Text -match "pid=(\d+)") {
        return [int]$Matches[1]
    }
    return $null
}

function Test-MatchingShuoRunning {
    param([string]$Exe, [int]$ProcessId)
    if ($ProcessId -le 0) {
        return $false
    }
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return $false
    }
    try {
        return $process.Path -eq $Exe
    } catch {
        return $false
    }
}

function Invoke-Shuo {
    param(
        [string]$Exe,
        [string[]]$Arguments,
        [string]$OutPath,
        [string]$ErrPath,
        [string]$WorkingDirectory,
        [int]$TimeoutMs = 30000
    )
    $process = Start-Process -FilePath $Exe `
        -ArgumentList $Arguments `
        -WorkingDirectory $WorkingDirectory `
        -RedirectStandardOutput $OutPath `
        -RedirectStandardError $ErrPath `
        -PassThru `
        -WindowStyle Hidden
    $exited = $process.WaitForExit($TimeoutMs)
    if (-not $exited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "shuo command timed out after $TimeoutMs ms: $($Arguments -join ' ')"
    }
    return $process.ExitCode
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

$failures = New-Object System.Collections.Generic.List[string]

try {
    $versionCode = Invoke-Shuo $exe @("--version") (Join-Path $LogDir "version.out.txt") (Join-Path $LogDir "version.err.txt") $root
    if ($versionCode -ne 0) {
        $failures.Add("version exited $versionCode")
    }

    $serviceStartCode = Invoke-Shuo $exe @("service", "start") (Join-Path $LogDir "service-start.out.txt") (Join-Path $LogDir "service-start.err.txt") $root
    if ($serviceStartCode -ne 0) {
        $failures.Add("service start exited $serviceStartCode")
    }

    $statusCode = Invoke-Shuo $exe @("service", "status") (Join-Path $LogDir "status.out.txt") (Join-Path $LogDir "status.err.txt") $root
    if ($statusCode -ne 0) {
        $failures.Add("service status exited $statusCode")
    }

    $statusOut = Get-Content (Join-Path $LogDir "status.out.txt") -Raw -ErrorAction SilentlyContinue
    $daemonPid = Read-DaemonPid $statusOut
    $daemonRunning = Test-MatchingShuoRunning $exe $daemonPid
    if (-not $daemonRunning) {
        $failures.Add("daemon was not running after service start")
    }

    $serviceStartAgainCode = Invoke-Shuo $exe @("service", "start") (Join-Path $LogDir "service-start-again.out.txt") (Join-Path $LogDir "service-start-again.err.txt") $root
    if ($serviceStartAgainCode -ne 0) {
        $failures.Add("second service start exited $serviceStartAgainCode")
    }

    $afterStartAgainStatusCode = Invoke-Shuo $exe @("service", "status") (Join-Path $LogDir "after-start-again-status.out.txt") (Join-Path $LogDir "after-start-again-status.err.txt") $root
    if ($afterStartAgainStatusCode -ne 0) {
        $failures.Add("after-start-again service status exited $afterStartAgainStatusCode")
    }
    $afterStartAgainStatusOut = Get-Content (Join-Path $LogDir "after-start-again-status.out.txt") -Raw -ErrorAction SilentlyContinue
    $afterStartAgainPid = Read-DaemonPid $afterStartAgainStatusOut
    if ($daemonPid -ne $null -and $afterStartAgainPid -ne $daemonPid) {
        $failures.Add("service start was not idempotent: pid changed from $daemonPid to $afterStartAgainPid")
    }

    $secondCode = Invoke-Shuo $exe @("--daemon") (Join-Path $LogDir "second-daemon.out.txt") (Join-Path $LogDir "second-daemon.err.txt") $root
    if ($secondCode -eq 0) {
        $failures.Add("second daemon started successfully")
    }

    $serviceRestartCode = Invoke-Shuo $exe @("service", "restart") (Join-Path $LogDir "service-restart.out.txt") (Join-Path $LogDir "service-restart.err.txt") $root
    if ($serviceRestartCode -ne 0) {
        $failures.Add("service restart exited $serviceRestartCode")
    }

    $afterRestartStatusCode = Invoke-Shuo $exe @("service", "status") (Join-Path $LogDir "after-restart-status.out.txt") (Join-Path $LogDir "after-restart-status.err.txt") $root
    if ($afterRestartStatusCode -ne 0) {
        $failures.Add("after-restart service status exited $afterRestartStatusCode")
    }
    $afterRestartStatusOut = Get-Content (Join-Path $LogDir "after-restart-status.out.txt") -Raw -ErrorAction SilentlyContinue
    $afterRestartPid = Read-DaemonPid $afterRestartStatusOut
    $afterRestartRunning = Test-MatchingShuoRunning $exe $afterRestartPid
    if (-not $afterRestartRunning) {
        $failures.Add("daemon was not running after service restart")
    }
    if ($daemonPid -ne $null -and $afterRestartPid -eq $daemonPid) {
        $failures.Add("service restart did not replace daemon pid $daemonPid")
    }

    $busyDir = Join-Path $LogDir "busy"
    New-Item -ItemType Directory -Path $busyDir -Force | Out-Null
    $jobs = 1..$ClientCount | ForEach-Object {
        $index = $_
        Start-Job -ScriptBlock {
            param($Exe, $Root, $BusyDir, $Index)
            $process = Start-Process -FilePath $Exe `
                -ArgumentList @("service", "status") `
                -WorkingDirectory $Root `
                -RedirectStandardOutput (Join-Path $BusyDir "client-$Index.out.txt") `
                -RedirectStandardError (Join-Path $BusyDir "client-$Index.err.txt") `
                -PassThru `
                -WindowStyle Hidden
            $exited = $process.WaitForExit(30000)
            if (-not $exited) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                Set-Content -Path (Join-Path $BusyDir "client-$Index.exit.txt") -Value 124 -Encoding ASCII
                exit 124
            }
            Set-Content -Path (Join-Path $BusyDir "client-$Index.exit.txt") -Value $process.ExitCode -Encoding ASCII
            exit $process.ExitCode
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

    $afterBusyStatusCode = Invoke-Shuo $exe @("service", "status") (Join-Path $LogDir "after-busy-status.out.txt") (Join-Path $LogDir "after-busy-status.err.txt") $root
    if ($afterBusyStatusCode -ne 0) {
        $failures.Add("after-busy service status exited $afterBusyStatusCode")
    }

    $daemonRunningBeforeStop = Test-MatchingShuoRunning $exe $afterRestartPid
    if (-not $daemonRunningBeforeStop) {
        $failures.Add("daemon was not running after smoke")
    }

    $serviceStopCode = Invoke-Shuo $exe @("service", "stop") (Join-Path $LogDir "service-stop.out.txt") (Join-Path $LogDir "service-stop.err.txt") $root
    if ($serviceStopCode -ne 0) {
        $failures.Add("service stop exited $serviceStopCode")
    }
    Start-Sleep -Milliseconds 500
    $daemonRunningAfterStop = Test-MatchingShuoRunning $exe $afterRestartPid
    if ($daemonRunningAfterStop) {
        $failures.Add("daemon was still running after service stop")
    }

    $afterStopStatusCode = Invoke-Shuo $exe @("service", "status") (Join-Path $LogDir "after-stop-status.out.txt") (Join-Path $LogDir "after-stop-status.err.txt") $root
    if ($afterStopStatusCode -ne 0) {
        $failures.Add("after-stop service status exited $afterStopStatusCode")
    }

    $summary = [ordered]@{
        exe = $exe
        log = $LogDir
        integrity = Get-IntegrityLabel
        version_exit = $versionCode
        service_start_exit = $serviceStartCode
        service_start_again_exit = $serviceStartAgainCode
        service_restart_exit = $serviceRestartCode
        daemon_pid = $daemonPid
        after_start_again_pid = $afterStartAgainPid
        after_restart_pid = $afterRestartPid
        daemon_running_before_stop = $daemonRunningBeforeStop
        status_exit = $statusCode
        after_start_again_status_exit = $afterStartAgainStatusCode
        second_daemon_exit = $secondCode
        after_restart_status_exit = $afterRestartStatusCode
        busy_total = $ClientCount
        busy_exit_files = $busyCodes.Count
        busy_exit_0 = $busyExit0
        busy_nonzero = $busyNonzero
        after_busy_status_exit = $afterBusyStatusCode
        service_stop_exit = $serviceStopCode
        daemon_running_after_stop = $daemonRunningAfterStop
        after_stop_status_exit = $afterStopStatusCode
        service_start_out = Get-Content (Join-Path $LogDir "service-start.out.txt") -Raw -ErrorAction SilentlyContinue
        service_start_again_out = Get-Content (Join-Path $LogDir "service-start-again.out.txt") -Raw -ErrorAction SilentlyContinue
        service_restart_out = Get-Content (Join-Path $LogDir "service-restart.out.txt") -Raw -ErrorAction SilentlyContinue
        status_out = $statusOut
        after_start_again_status_out = $afterStartAgainStatusOut
        second_daemon_err = Get-Content (Join-Path $LogDir "second-daemon.err.txt") -Raw -ErrorAction SilentlyContinue
        after_restart_status_out = $afterRestartStatusOut
        after_busy_status_out = Get-Content (Join-Path $LogDir "after-busy-status.out.txt") -Raw -ErrorAction SilentlyContinue
        service_stop_out = Get-Content (Join-Path $LogDir "service-stop.out.txt") -Raw -ErrorAction SilentlyContinue
        after_stop_status_out = Get-Content (Join-Path $LogDir "after-stop-status.out.txt") -Raw -ErrorAction SilentlyContinue
        failures = @($failures)
    }

    $summary | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $LogDir "summary.json") -Encoding UTF8
    $summary | ConvertTo-Json -Depth 5

    if ($failures.Count -gt 0) {
        exit 1
    }
} finally {
    Stop-MatchingShuo $exe
}
