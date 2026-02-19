$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$mainPy = Join-Path $projectRoot "main.py"
$outFile = Join-Path $projectRoot "src/env_contract.rs"

if (-not (Test-Path $mainPy)) {
    throw "main.py not found at $mainPy"
}

$txt = Get-Content -Raw -Path $mainPy
$patterns = @(
    'os\.getenv\("([A-Z0-9_]+)"',
    'env_bool\("([A-Z0-9_]+)"',
    'env_float\("([A-Z0-9_]+)"',
    'env_int\("([A-Z0-9_]+)"',
    '_env_int\("([A-Z0-9_]+)"',
    '_env_float\("([A-Z0-9_]+)"',
    'os\.environ\["([A-Z0-9_]+)"\]'
)

$keys = New-Object System.Collections.Generic.List[string]
foreach ($p in $patterns) {
    $regex = [regex]$p
    foreach ($m in $regex.Matches($txt)) {
        $keys.Add($m.Groups[1].Value)
    }
}

$uniq = $keys | Sort-Object -Unique

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("pub const ENV_CONTRACT_KEYS: &[&str] = &[")
foreach ($k in $uniq) {
    $lines.Add("    `"$k`",")
}
$lines.Add("];")

[System.IO.File]::WriteAllLines($outFile, $lines)
Write-Host "Wrote $($uniq.Count) keys to $outFile"
