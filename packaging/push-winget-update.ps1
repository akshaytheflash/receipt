$ErrorActionPreference = "Stop"
$gh = "C:\Program Files\GitHub CLI\gh.exe"
$fork = "akshaytheflash/winget-pkgs"
$branch = "akshaytheflash.receipt-0.1.0"
$tmp = Join-Path $env:TEMP "winget-pr"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Api($method, $path, $obj) {
    $f = Join-Path $tmp "req.json"
    [IO.File]::WriteAllText($f, ($obj | ConvertTo-Json -Depth 20 -Compress), [Text.UTF8Encoding]::new($false))
    $out = & $gh api -X $method $path --input $f 2>&1
    if ($LASTEXITCODE -ne 0) { throw "$method $path failed: $out" }
    return (($out -join "`n") | ConvertFrom-Json)
}

$head = (& $gh api "repos/$fork/git/ref/heads/$branch" --jq ".object.sha")
Write-Output "base $head"

$root = "C:\Users\aksha\receipt\packaging\winget\manifests\a\akshaytheflash\receipt\0.1.0"
$entries = @()
foreach ($f in Get-ChildItem $root -File) {
    $b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($f.FullName))
    $blob = Api "POST" "repos/$fork/git/blobs" @{ content = $b64; encoding = "base64" }
    Write-Output "blob $($f.Name) $($blob.sha)"
    $entries += @{
        path = "manifests/a/akshaytheflash/receipt/0.1.0/$($f.Name)"
        mode = "100644"
        type = "blob"
        sha  = $blob.sha
    }
}

$tree = Api "POST" "repos/$fork/git/trees" @{ base_tree = $head; tree = $entries }
$commit = Api "POST" "repos/$fork/git/commits" @{
    message = "Fix manifest: schema 1.12.0 and schema headers"
    tree    = $tree.sha
    parents = @($head)
}
Write-Output "commit $($commit.sha)"

$ref = Api "PATCH" "repos/$fork/git/refs/heads/$branch" @{ sha = $commit.sha; force = $true }
Write-Output "ref $($ref.ref) -> $($ref.object.sha)"
