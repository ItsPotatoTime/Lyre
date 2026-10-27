# Collects what a Windows "Open with" implementation has to read, so it can be
# written against your actual machine instead of against a guess.
#
# Read-only: it queries the registry and prints. It changes nothing.
#
# Run from the repo root:
#     powershell -ExecutionPolicy Bypass -File scripts\win-openwith-dump.ps1
#
# It writes build\openwith-dump.txt, which is on the shared disk, so the Linux
# side can read it without copying anything.

$ErrorActionPreference = 'SilentlyContinue'

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$outDir = Join-Path $root 'build'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$out = Join-Path $outDir 'openwith-dump.txt'

# A jar is what failed on Linux, a txt is the simplest common case, and a png
# exercises an app-registered type rather than a ProgID one.
$extensions = @('.jar', '.txt', '.png', '.pdf')

function Section($title) {
    Add-Content -Path $out -Value ''
    Add-Content -Path $out -Value ('=' * 70)
    Add-Content -Path $out -Value $title
    Add-Content -Path $out -Value ('=' * 70)
}

function Query($key) {
    Add-Content -Path $out -Value "--- reg query `"$key`" ---"
    $result = & reg query $key /s 2>&1
    if ($LASTEXITCODE -ne 0) {
        Add-Content -Path $out -Value '(not present)'
    } else {
        $result | Add-Content -Path $out
    }
}

Set-Content -Path $out -Value "Lyre open-with dump"
Add-Content -Path $out -Value "machine: $env:COMPUTERNAME"
Add-Content -Path $out -Value "user: $env:USERNAME"
Add-Content -Path $out -Value "windows: $([System.Environment]::OSVersion.VersionString)"
Add-Content -Path $out -Value "powershell: $($PSVersionTable.PSVersion)"

Section 'assoc / ftype'
foreach ($ext in $extensions) {
    Add-Content -Path $out -Value "--- assoc $ext ---"
    $a = & cmd /c "assoc $ext" 2>&1
    Add-Content -Path $out -Value ($a -join ' ')
    $progId = ($a -join '') -replace '^.*=', ''
    if ($progId -and $progId -notmatch '^\[?File Association Not Found') {
        Add-Content -Path $out -Value "--- ftype $progId ---"
        $f = & cmd /c "ftype $progId" 2>&1
        Add-Content -Path $out -Value ($f -join ' ')
    }
}

Section 'Per-extension registry'
foreach ($ext in $extensions) {
    Query "HKEY_CLASSES_ROOT\$ext"
    Query "HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext"
}

Section 'ProgID open commands'
$progIds = @()
foreach ($ext in $extensions) {
    $a = (& cmd /c "assoc $ext" 2>&1) -join ''
    if ($a -match '=(.+)$') { $progIds += $Matches[1].Trim() }
}
foreach ($progId in ($progIds | Sort-Object -Unique)) {
    if (-not $progId) { continue }
    Query "HKEY_CLASSES_ROOT\$progId\shell"
}

Section 'Applications registered for "Open with"'
Query 'HKEY_CLASSES_ROOT\Applications' | Out-Null
# The whole Applications tree is enormous; list the names only, then the open
# command for the first forty, which is enough to see the shape.
$apps = Get-ChildItem 'Registry::HKEY_CLASSES_ROOT\Applications' | Select-Object -First 40
foreach ($app in $apps) {
    $cmd = "HKEY_CLASSES_ROOT\Applications\$($app.PSChildName)\shell\open\command"
    $value = (Get-ItemProperty -Path "Registry::$cmd" -Name '(default)').'(default)'
    if ($value) {
        Add-Content -Path $out -Value "$($app.PSChildName)  =>  $value"
    }
}

Section 'RegisteredApplications with file capabilities'
$registered = Get-ItemProperty 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\RegisteredApplications'
foreach ($prop in $registered.PSObject.Properties) {
    if ($prop.Name -like 'PS*') { continue }
    $path = "HKEY_LOCAL_MACHINE\SOFTWARE\$($prop.Value)"
    Add-Content -Path $out -Value "$($prop.Name)  =>  $($prop.Value)"
    $caps = & reg query $path /s 2>&1
    if ($LASTEXITCODE -eq 0) {
        $caps | Select-String -Pattern 'FileAssociations|\.jar|\.txt|\.png|\.pdf' |
            Select-Object -First 12 |
            ForEach-Object { Add-Content -Path $out -Value "    $_" }
    }
}

Section 'What Explorer would use to open these files right now'
foreach ($ext in $extensions) {
    $key = "HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice"
    $value = (Get-ItemProperty -Path "Registry::$key" -Name 'ProgId').ProgId
    Add-Content -Path $out -Value "$ext  ->  $(if ($value) { $value } else { '(no UserChoice)' })"
}

Add-Content -Path $out -Value ''
Add-Content -Path $out -Value "written to $out"
Write-Host "Wrote $out"
