# Dumps the mainnet programs the LiteSVM tests load as fixtures. The .so files are
# gitignored (~3 MB); run this once after cloning, and again whenever a fixture
# should track a newer on-chain deployment.
#
#   .\scripts\fetch-fixtures.ps1            # mainnet-beta
#   .\scripts\fetch-fixtures.ps1 -Url <rpc> # custom RPC
#
# Needs the Solana CLI (`solana program dump`).

param(
    [string]$Url = "https://api.mainnet-beta.solana.com"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dir = Join-Path $root "programs\basket\tests\fixtures"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$fixtures = @(
    # Meteora DAMM v2 (cp-amm): pool + position CPI target for seed / mint / redeem.
    @{ Name = "cp_amm.so";             Id = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG" },
    # Metaplex Token Metadata: share-mint metadata written at create_basket.
    @{ Name = "mpl_token_metadata.so"; Id = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s" }
)

foreach ($f in $fixtures) {
    $out = Join-Path $dir $f.Name
    Write-Host "dumping $($f.Id) -> $out"
    solana program dump --url $Url $f.Id $out
    if ($LASTEXITCODE -ne 0) { throw "solana program dump failed for $($f.Id)" }
}

Write-Host "done"
