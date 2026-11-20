# Builds the Android app of Lorelamp (apps/mobile) as a signed release APK for arm64, named after
# the version, and adds its checksum to SHA256SUMS.txt. Publishes nothing.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release-android.ps1 [-OutDir <folder>]
#
# Default output: target\release-assets\<version>\ in this repo.
#
# Needs: Node, Rust with the target aarch64-linux-android, JDK 17+ (JAVA_HOME), the Android SDK
# (ANDROID_HOME, default %LOCALAPPDATA%\Android\Sdk) with build-tools and an NDK (NDK_HOME,
# default: the newest one in the SDK's ndk folder).
#
# The signing key never lives in the repository. It is taken from
#   LORELAMP_SIGNING_STORE (PKCS12 file) + LORELAMP_SIGNING_PASSWORD [+ LORELAMP_SIGNING_ALIAS], or
#   a folder with lorelamp-android.p12 and password.txt: LORELAMP_SIGNING_DIR, by default the
#   folder "lorelamp-signing" next to the repository.
# Make a key once with:
#   keytool -genkeypair -storetype PKCS12 -keystore lorelamp-android.p12 -alias lorelamp `
#     -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Lorelamp"
# Keep it: Android only installs an update over an app signed with the same key.
[CmdletBinding()]
param([string]$OutDir)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$mobile = Join-Path $repo 'apps\mobile'

function Invoke-Step([string]$what, [scriptblock]$run) {
    Write-Host "== $what"
    & $run
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}

# 1. One version everywhere.
$conf = Get-Content (Join-Path $mobile 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = $conf.version
$product = $conf.productName
$found = [ordered]@{
    'apps/mobile tauri.conf.json' = $version
    'Cargo.toml'                  = ([regex]::Match((Get-Content (Join-Path $repo 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"')).Groups[1].Value
    'apps/mobile/package.json'    = (Get-Content (Join-Path $mobile 'package.json') -Raw | ConvertFrom-Json).version
}
$wrong = $found.GetEnumerator() | Where-Object { $_.Value -ne $version }
if ($wrong) { throw "Versions differ: $(($found.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', ')" }
Write-Host "$product $version (Android)"

# 2. Android SDK and NDK.
if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
if (-not (Test-Path $env:ANDROID_HOME)) { throw "Android SDK not found: set ANDROID_HOME." }
if (-not $env:NDK_HOME) {
    $ndk = Get-ChildItem (Join-Path $env:ANDROID_HOME 'ndk') -Directory -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Name } | Select-Object -Last 1
    if (-not $ndk) { throw "No NDK in $($env:ANDROID_HOME)\ndk: install one or set NDK_HOME." }
    $env:NDK_HOME = $ndk.FullName
}
$buildTools = Get-ChildItem (Join-Path $env:ANDROID_HOME 'build-tools') -Directory | Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $buildTools) { throw "No build-tools in the Android SDK." }

# 3. Signing key (see the head of this file). The password is only ever passed on in the
#    environment of this process, never printed.
if (-not $env:LORELAMP_SIGNING_STORE -or -not $env:LORELAMP_SIGNING_PASSWORD) {
    $dir = $env:LORELAMP_SIGNING_DIR
    if (-not $dir) {
        # Next to the repository; from a git worktree that is next to the main checkout.
        $common = (& git -C $repo rev-parse --path-format=absolute --git-common-dir).Trim()
        $dir = Join-Path (Split-Path -Parent (Split-Path -Parent $common)) 'lorelamp-signing'
    }
    $store = Join-Path $dir 'lorelamp-android.p12'
    $passFile = Join-Path $dir 'password.txt'
    if (-not (Test-Path -LiteralPath $store) -or -not (Test-Path -LiteralPath $passFile)) {
        throw 'No signing key: set LORELAMP_SIGNING_STORE and LORELAMP_SIGNING_PASSWORD, or LORELAMP_SIGNING_DIR (lorelamp-android.p12 + password.txt).'
    }
    $env:LORELAMP_SIGNING_STORE = $store
    $env:LORELAMP_SIGNING_PASSWORD = (Get-Content -LiteralPath $passFile -Raw).Trim()
}
if (-not (Test-Path -LiteralPath $env:LORELAMP_SIGNING_STORE)) { throw 'LORELAMP_SIGNING_STORE does not exist.' }

if (-not $OutDir) { $OutDir = Join-Path $repo "target\release-assets\$version" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# 4. Build. Gradle signs the APK with the key from the environment (gen/android/app/build.gradle.kts).
$apk = Join-Path $mobile 'src-tauri\gen\android\app\build\outputs\apk\arm64\release\app-arm64-release.apk'
Remove-Item -LiteralPath $apk -ErrorAction SilentlyContinue
$android = Join-Path $mobile 'src-tauri\gen\android'
Push-Location $mobile
try {
    Invoke-Step 'npm ci (mobile)' { npm ci --no-audit --no-fund }
    Write-Host '== tauri android build'
    $log = Join-Path $OutDir 'android-build.log'
    # Through cmd: the tools write progress to stderr, which must not end this script.
    cmd /c "npx tauri android build --apk --target aarch64 --split-per-abi --ci > `"$log`" 2>&1"
    $failed = $LASTEXITCODE -ne 0
    Get-Content $log -Tail 4
    if ($failed) {
        # The Tauri CLI places the compiled library in the Android project by symbolic link, which
        # Windows only allows with developer mode. Without it the compile has succeeded and only
        # that last step failed: copy the library instead and let Gradle finish without its Rust step.
        if (-not (Select-String -Path $log -Pattern 'symbolic link' -Quiet)) { throw "tauri android build failed, see $log" }
        Write-Host '== no symbolic links on this PC: copying the library, then Gradle'
        $so = Join-Path $repo 'target\aarch64-linux-android\release\librp_mobile_lib.so'
        if (-not (Test-Path $so)) { throw "Library not found: $so" }
        $jni = Join-Path $android 'app\src\main\jniLibs\arm64-v8a'
        New-Item -ItemType Directory -Force $jni | Out-Null
        Copy-Item $so $jni -Force
        Invoke-Step 'gradle assemble' { cmd /c "cd /d `"$android`" && .\gradlew.bat assembleArm64Release -x rustBuildArm64Release --console=plain >> `"$log`" 2>&1" }
    }
    Remove-Item $log -ErrorAction SilentlyContinue
} finally { Pop-Location }
if (-not (Test-Path $apk)) { throw "Signed APK not found: $apk" }

# 5. The signature must hold before anything is copied.
Invoke-Step 'apksigner verify' { & (Join-Path $buildTools.FullName 'apksigner.bat') verify --verbose --print-certs $apk | Select-String -Pattern '^Verifies|Verified using v[23]|certificate SHA-256|certificate DN' }

# 6. Copy under the release name; replace only this file's line in SHA256SUMS.txt.
$name = "$product-$version.apk"
$dest = Join-Path $OutDir $name
Copy-Item $apk $dest -Force
$hash = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLowerInvariant()
$sumsFile = Join-Path $OutDir 'SHA256SUMS.txt'
$sums = @()
if (Test-Path $sumsFile) { $sums = @(Get-Content $sumsFile | Where-Object { $_ -and ($_ -notmatch "  $([regex]::Escape($name))$") }) }
$sums += "$hash  $name"
[IO.File]::WriteAllLines($sumsFile, [string[]]$sums)

# 7. Show what was built.
$badging = & (Join-Path $buildTools.FullName 'aapt2.exe') dump badging $dest
$min = ([regex]::Match(($badging -join "`n"), "(?:minSdkVersion|sdkVersion):'(\d+)'")).Groups[1].Value
'{0}  {1:N1} MB  minSdk={2}' -f $name, ((Get-Item $dest).Length / 1MB), $min
"$hash  $name"
