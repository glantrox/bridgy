# Install the command only. Agent handoffs are enabled separately with bridgy install.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [string]$BinDir = (Join-Path $env:LOCALAPPDATA 'Programs\Bridgy\bin')
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "Executable not found: $Binary"
}
if (-not [IO.Path]::IsPathRooted($BinDir)) {
    throw 'BinDir must be an absolute path'
}
$null = New-Item -ItemType Directory -Path $BinDir -Force
$target = Join-Path $BinDir 'bridgy.exe'
if ([IO.Path]::GetFullPath($Binary) -ne [IO.Path]::GetFullPath($target)) {
    Copy-Item -LiteralPath $Binary -Destination $target -Force
}
Write-Output "Installed command: $target"
$onPath = @($env:Path -split ';' | ForEach-Object { $_.TrimEnd('\') }) -contains $BinDir.TrimEnd('\')
if (-not $onPath) {
    $quoted = "'" + $BinDir.Replace("'", "''") + "'"
    Write-Output 'This directory is not on PATH. Run these PowerShell commands:'
    Write-Output ('  $env:Path = ' + $quoted + " + ';' + `$env:Path")
    Write-Output ("  [Environment]::SetEnvironmentVariable('Path', " + $quoted + " + ';' + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')")
}
Write-Output 'Open a fresh terminal, then run: bridgy --version'
Write-Output 'Launch the browser: bridgy'
Write-Output 'Optional agent handoffs: bridgy install'
