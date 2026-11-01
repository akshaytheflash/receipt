$ErrorActionPreference = "Stop"
$gh = "C:\Program Files\GitHub CLI\gh.exe"
$fork = "akshaytheflash/winget-pkgs"
$upstream = "microsoft/winget-pkgs"
$branch = "akshaytheflash.receipt-0.1.0"
$tmp = Join-Path $env:TEMP "winget-pr"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Api-Post($path, $obj) {
    $f = Join-Path $tmp "req.json"
    [IO.File]::WriteAllText($f, ($obj | ConvertTo-Json -Depth 20 -Compress), [Text.UTF8Encoding]::new($false))
    $out = & $gh api -X POST $path --input $f 2>&1
    if ($LASTEXITCODE -ne 0) { throw "POST $path failed: $out" }
    return (($out -join "`n") | ConvertFrom-Json)
}

$head = (& $gh api "repos/$fork/git/ref/heads/master" --jq ".object.sha")
Write-Output "base $head"

$root = "C:\Users\aksha\receipt\packaging\winget\manifests\a\akshaytheflash\receipt\0.1.0"
$entries = @()
foreach ($f in Get-ChildItem $root -File) {
    $b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($f.FullName))
    $blob = Api-Post "repos/$fork/git/blobs" @{ content = $b64; encoding = "base64" }
    Write-Output "blob $($f.Name) $($blob.sha)"
    $entries += @{
        path = "manifests/a/akshaytheflash/receipt/0.1.0/$($f.Name)"
        mode = "100644"
        type = "blob"
        sha  = $blob.sha
    }
}

$tree = Api-Post "repos/$fork/git/trees" @{ base_tree = $head; tree = $entries }
Write-Output "tree $($tree.sha)"

$commit = Api-Post "repos/$fork/git/commits" @{
    message = "New package: akshaytheflash.receipt version 0.1.0"
    tree    = $tree.sha
    parents = @($head)
}
Write-Output "commit $($commit.sha)"

$ref = Api-Post "repos/$fork/git/refs" @{ ref = "refs/heads/$branch"; sha = $commit.sha }
Write-Output "ref $($ref.ref)"

$body = @"
- [x] Have you signed the [Contributor License Agreement](https://cla.opensource.microsoft.com/microsoft/winget-pkgs)?

###### Microsoft Reviewers: [Open in CodeFlow](https://microsoft.github.io/codeflow/)
"@

$pr = Api-Post "repos/$upstream/pulls" @{
    title = "[New package] akshaytheflash.receipt version 0.1.0"
    head  = "akshaytheflash:$branch"
    base  = "master"
    body  = $body
}
Write-Output "PR $($pr.html_url)"
