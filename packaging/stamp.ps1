# Stamps the release binary hash into the scoop and winget manifests and
# copies the binary into the npm package. Run after the GitHub release exists.
#
#   .\packaging\stamp.ps1 -Exe .\dist\receipt-x86_64-pc-windows-msvc.exe

param(
    [Parameter(Mandatory = $true)]
    [string]$Exe
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Exe)) {
    throw "no such file: $Exe"
}

$hash = (Get-FileHash $Exe -Algorithm SHA256).Hash.ToLower()
Write-Output "sha256 $hash"

$scoop = Join-Path $PSScriptRoot "scoop\receipt.json"
(Get-Content $scoop -Raw) -replace '"hash": "[0-9a-f]*"', "`"hash`": `"$hash`"" |
    Set-Content $scoop -Encoding utf8 -NoNewline
Write-Output "updated $scoop"

$winget = Join-Path $PSScriptRoot "winget\manifests\a\akshaytheflash\receipt\0.1.0\akshaytheflash.receipt.installer.yaml"
(Get-Content $winget -Raw) -replace 'InstallerSha256: .*', "InstallerSha256: $hash" |
    Set-Content $winget -Encoding utf8 -NoNewline
Write-Output "updated $winget"

$npmBin = Join-Path $PSScriptRoot "..\npm\bin\receipt.exe"
Copy-Item $Exe $npmBin -Force
Write-Output "copied binary to $npmBin"
