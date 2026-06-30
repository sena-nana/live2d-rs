param(
  [string]$SessionPayload = "",
  [string]$Addr = ""
)

$ErrorActionPreference = "Stop"

$manifest = Join-Path $PSScriptRoot "Cargo.toml"
$cargoArgs = @("run", "--manifest-path", $manifest, "--", "replay")

if ($SessionPayload -ne "") {
  $cargoArgs += $SessionPayload
}

if ($Addr -ne "") {
  $env:NANAVTS_DISPLAY_ADDR = $Addr
}

cargo @cargoArgs
