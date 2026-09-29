param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$ExpectedVersion = '0.1.0-alpha.1'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$exe = (Resolve-Path -LiteralPath $Executable).Path
$names = @('GL_DATABASE_URL', 'GL_AUTHOR', 'GL_STATEMENT_TIMEOUT_SECS', 'GL_API_TOKEN')
$saved = @{}
foreach ($name in $names) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    Remove-Item -LiteralPath "Env:\$name" -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath "Env:\$name") { throw "Could not clear $name" }
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('geoledger-smoke-' + [guid]::NewGuid().ToString('N'))
$repo = Join-Path $root ('spaces ' + [char]0x5730 + [char]0x56fe)
function Invoke-GlJson {
    param([string[]]$Arguments)
    $result = & $exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "gl failed: $Arguments" }
    return ConvertFrom-Json -InputObject ($result -join [Environment]::NewLine)
}
try {
    $version = & $exe --version
    if ($LASTEXITCODE -ne 0 -or $version -ne "gl $ExpectedVersion") {
        throw "Unexpected CLI version: $version"
    }
    $help = (& $exe --help) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $help -notmatch 'mapseekai') { throw 'CLI help/default author check failed' }
    $init = Invoke-GlJson -Arguments @('--repo', $repo, 'init')
    if ($init.format_version -ne 3) { throw 'Unexpected repository format' }
    $history = Invoke-GlJson -Arguments @('--repo', $repo, 'log')
    if ($history.commits[0].commit.author -ne 'mapseekai') { throw 'Unexpected default author' }
    $null = Invoke-GlJson -Arguments @('--repo', $repo, 'branch', 'windows-test')
    $status = Invoke-GlJson -Arguments @('--repo', $repo, 'status')
    if (-not $status.clean) { throw 'Expected a clean working copy' }
    $check = Invoke-GlJson -Arguments @('--repo', $repo, 'fsck')
    if (-not $check.ok) { throw 'Repository integrity check failed' }
    if (-not (Test-Path -LiteralPath (Join-Path $repo '.geoledger/repository.sqlite'))) {
        throw 'Repository file is missing'
    }
    [ordered]@{version=$version; format=3; author='mapseekai'; unicode_path=$true; fsck=$check.ok} | ConvertTo-Json
}
finally {
    foreach ($name in $names) {
        if ($null -eq $saved[$name]) {
            Remove-Item -LiteralPath "Env:\$name" -ErrorAction SilentlyContinue
        } else {
            Set-Item -LiteralPath "Env:\$name" -Value $saved[$name]
        }
    }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
