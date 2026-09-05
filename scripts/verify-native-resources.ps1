param([int]$Iterations = 10000, [ValidateRange(1, 7200)][int]$TimeoutSeconds = 900)
$ErrorActionPreference = 'Stop'
if ($Iterations -lt 100 -or $Iterations -gt 100000) { throw 'Iterations must be 100..100000' }
$runDirectory = Join-Path (Get-Location) ('target/verification/resources-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $runDirectory | Out-Null
$build = & cargo +1.85.0 test --locked -p windows-task --all-features --test windows_smoke --no-run --message-format=json 2> (Join-Path $runDirectory 'build.stderr.log')
$build | Set-Content (Join-Path $runDirectory 'build.jsonl')
if ($LASTEXITCODE -ne 0) { throw "Build failed: $runDirectory" }
$artifact = $build | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'windows_smoke' -and $_.executable } | Select-Object -Last 1
if (!$artifact) { throw "No test executable: $runDirectory" }
$previousIterations = $env:WINDOWS_TASK_SESSION_ITERATIONS
$samples = [System.Collections.Generic.List[object]]::new()
try {
    $env:WINDOWS_TASK_SESSION_ITERATIONS = "$Iterations"
    $process = Start-Process -FilePath $artifact.executable -ArgumentList @('repeated_sessions_confirm_shutdown_and_reject_new_work', '--exact', '--test-threads=1', '--nocapture') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runDirectory 'test.stdout.log') -RedirectStandardError (Join-Path $runDirectory 'test.stderr.log')
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    while (!$process.HasExited) {
        $process.Refresh()
        if ($timer.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            $timedOut = $true
            # This is our disposable test process, never a library COM worker.
            $process.Kill($true)
            $process.WaitForExit()
            break
        }
        if (!$process.HasExited) {
            $samples.Add([pscustomobject]@{ milliseconds = $timer.ElapsedMilliseconds; handles = $process.HandleCount; threads = $process.Threads.Count; private_bytes = $process.PrivateMemorySize64 })
        }
        Start-Sleep -Milliseconds 100
    }
    $process.WaitForExit()
    $samples | ConvertTo-Json | Set-Content (Join-Path $runDirectory 'samples.json')
    # Ignore loader/COM warm-up and compare equal windows while the process lives.
    $steady = @($samples | Where-Object { $_.milliseconds -ge 2000 })
    if ($timedOut -or $steady.Count -lt 20) {
        @{ revision = (& git rev-parse HEAD); iterations = $Iterations; timed_out = $timedOut; timeout_seconds = $TimeoutSeconds; elapsed_ms = $timer.ElapsedMilliseconds; exit_code = $process.ExitCode; steady_samples = $steady.Count; outcome = 'failed'; reproduce = "pwsh -File scripts/verify-native-resources.ps1 -Iterations $Iterations -TimeoutSeconds $TimeoutSeconds" } | ConvertTo-Json | Set-Content (Join-Path $runDirectory 'results.json')
        throw "Native resource verification timed out or collected insufficient samples: $runDirectory"
    }
    $first = @($steady | Select-Object -First 10)
    $last = @($steady | Select-Object -Last 10)
    $growth = @{}
    foreach ($field in @('handles', 'threads', 'private_bytes')) {
        $growth[$field] = ($last | Measure-Object $field -Average).Average - ($first | Measure-Object $field -Average).Average
    }
    $report = @{ revision = (& git rev-parse HEAD); dirty = @(& git status --porcelain); iterations = $Iterations; exit_code = $process.ExitCode; growth = $growth; limits = @{ handles = 32; threads = 8; private_bytes = 67108864 }; reproduce = "pwsh -File scripts/verify-native-resources.ps1 -Iterations $Iterations" }
    $report | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $runDirectory 'results.json')
    if ($process.ExitCode -ne 0 -or $growth.handles -gt 32 -or $growth.threads -gt 8 -or $growth.private_bytes -gt 67108864) { throw "Native resource regression: $runDirectory" }
    Write-Output "Native resource evidence: $runDirectory"
} finally {
    $env:WINDOWS_TASK_SESSION_ITERATIONS = $previousIterations
}
