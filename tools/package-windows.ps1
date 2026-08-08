param([Parameter(Mandatory=$true)][string]$Output, [Parameter(Mandatory=$true)][string]$Version)
$ErrorActionPreference = "Stop"
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$') { throw "invalid semantic version" }
if (Test-Path $Output) { throw "release output already exists" }
if ((git status --porcelain).Length -ne 0) { throw "packaging requires a clean worktree" }
$revision = git rev-parse HEAD
$stage = Join-Path $env:RUNNER_TEMP "atrinik-client-windows-stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Path $stage, $Output | Out-Null
$env:ATRINIK_RUST_VERSION = "rust-1.97.1"
$env:ATRINIK_VERSION = $Version
cargo auditable build --locked --release --package atrinik-client
Copy-Item target/release/atrinik-client.exe, LICENSE, PROVENANCE.md, THIRD_PARTY_NOTICES.md -Destination $stage
Get-ChildItem $stage | ForEach-Object { $_.LastWriteTimeUtc = [DateTime]::new(1980,1,1,0,0,0,[DateTimeKind]::Utc) }
$archive = Join-Path $Output "atrinik-client-$Version-windows-amd64.zip"
Compress-Archive -Path "$stage/*" -DestinationPath $archive -CompressionLevel Optimal
syft "dir:$stage" --source-name atrinik-client --source-version $Version --output "cyclonedx-json=$(Join-Path $Output "atrinik-client-$Version-windows-amd64.sbom.cdx.json")"
$sbom = Get-Content (Join-Path $Output "atrinik-client-$Version-windows-amd64.sbom.cdx.json") | ConvertFrom-Json
if ($sbom.components.Count -lt 10) { throw "release SBOM is missing the effective Rust graph" }
[ordered]@{schema_version=1;version=$Version;revision=$revision;target="x86_64-pc-windows-msvc";rust=(rustc --version);sdl="3.4.14 static";protocol="game-protocol-1";renderer="scene-snapshot-1";symbols="stripped public package; private symbol packages begin in M6"} | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $Output "atrinik-client-$Version-windows-amd64.provenance.json")
Get-ChildItem $Output -File | Sort-Object Name | ForEach-Object { "{0}  {1}" -f (Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLowerInvariant(), $_.Name } | Set-Content -Encoding ascii (Join-Path $Output "atrinik-client-$Version-windows-amd64.SHA256SUMS")
Remove-Item -Recurse -Force $stage
