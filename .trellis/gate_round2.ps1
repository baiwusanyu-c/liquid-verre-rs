# Round-2 regression gate after Up-housekeeping fix (ASCII-only)
$ErrorActionPreference = 'Continue'
$env:PYTHONIOENCODING = 'utf-8'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = 'D:/github-project/liquid-verre-rs'
Set-Location $root
$E = Join-Path $root '.trellis/tasks/10-23-input-sink/gate-round2'
New-Item -ItemType Directory -Force -Path $E | Out-Null
"=== round2 gate run start $(Get-Date -Format o)" | Out-File -Encoding utf8 (Join-Path $E 'summary.txt')

function Step($name, [ScriptBlock]$block) {
  $log = Join-Path $E ($name + '.log')
  $t0 = Get-Date
  Write-Host (">> " + $name)
  try { & $block *>&1 | Out-File -Encoding utf8 $log } catch { $_ | Out-File -Encoding utf8 -Append $log }
  $code = $LASTEXITCODE
  $sec = [int]((Get-Date) - $t0).TotalSeconds
  ("{0} exit={1} {2}s" -f $name, $code, $sec) | Add-Content -Encoding utf8 (Join-Path $E 'summary.txt')
}

Step '01_cargo_test'  { cargo test --release --workspace }
Step '02_clippy'      { cargo clippy --workspace --all-targets --release -- -D warnings }
Step '03_fmt'         { cargo fmt --all -- --check }
Step '04_check_mjs'   { node scripts/check.mjs }
Step '05_sink_behavior' { python tools/verify_input_sink.py }

$exe = Join-Path $root 'target/release/liquid-verre-playground.exe'
$old = Join-Path $root '.trellis/tasks/10-23-glass-window-host-capabilities/baseline-evidence/exe_0.4.2-playground.exe'
$env:VERRE_EXE = $exe
$env:VERRE_OLD_EXE = $old

Step '06_cmp_visual'        { python tools/cmp_visual.py }
Step '07_verify_dirty_r1'   { python tools/verify_dirty.py }
Step '08_verify_dirty_r2'   { python tools/verify_dirty.py }
Step '09_verify_dirty_r3'   { python tools/verify_dirty.py }
Step '10_mouse_isolation'   { python tools/verify_mouse_isolation.py }
Step '11_validity'          { python tools/verify_validity.py }
Step '12_tremble_far'       { python tools/repro_tremble.py far }
Step '13_tremble_many'      { python tools/repro_tremble.py many }
Step '14_adaptive'          { python tools/verify_adaptive.py }
Step '15_gate_ab'           { python tools/gate_perf.py }

foreach ($b in @('cv_single_now.bmp','cv_many_now.bmp')) {
  $p = Join-Path $root ('tools/' + $b)
  if (Test-Path $p) { Copy-Item $p $E -Force }
}
Copy-Item $exe (Join-Path $E 'exe_candidate.exe') -Force
"=== round2 gate run done $(Get-Date -Format o)" | Add-Content -Encoding utf8 (Join-Path $E 'summary.txt')
Write-Host 'ALL DONE'
