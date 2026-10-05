#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root_dir"
if [[ "$(uname -s)" != Darwin ]]; then
    echo "macOS is required to create the application bundle." >&2
    exit 1
fi
command -v macdeployqt >/dev/null || { echo "Add the Qt bin directory to PATH (macdeployqt is required)." >&2; exit 1; }

target="${1:-}"
build_args=(--release --locked)
release_dir="$root_dir/target/release"
if [[ -n "$target" ]]; then
    build_args+=(--target "$target")
    release_dir="$root_dir/target/$target/release"
fi
cargo build "${build_args[@]}"

app_dir="$release_dir/bundle/macos/Tailer.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
rm -rf "$app_dir"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
cp "$release_dir/tailer" "$app_dir/Contents/MacOS/tailer"
cp "$root_dir/icons/icon.icns" "$app_dir/Contents/Resources/icon.icns"
cp "$root_dir/LICENSE" "$root_dir/THIRD_PARTY_NOTICES.md" "$app_dir/Contents/Resources/"
cp -R "$root_dir/licenses" "$app_dir/Contents/Resources/licenses"
cat > "$app_dir/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Tailer</string>
    <key>CFBundleDisplayName</key><string>Tailer</string>
    <key>CFBundleIdentifier</key><string>app.tailer.desktop</string>
    <key>CFBundleExecutable</key><string>tailer</string>
    <key>CFBundleIconFile</key><string>icon.icns</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
printf 'APPL????' > "$app_dir/Contents/PkgInfo"
# Only deploy the platform plugins we use. Deploying every SQL driver would
# pull in optional external database libraries that Tailer does not require.
qt_plugins="$(qmake -query QT_INSTALL_PLUGINS)"
mkdir -p "$app_dir/Contents/PlugIns/platforms"
cp "$qt_plugins/platforms/libqcocoa.dylib" "$app_dir/Contents/PlugIns/platforms/"
cp "$qt_plugins/platforms/libqoffscreen.dylib" "$app_dir/Contents/PlugIns/platforms/"
# QML is embedded in the executable, so explicitly scan the source imports.
macdeployqt "$app_dir" -qmldir="$root_dir/src" -no-plugins -no-codesign \
    -executable="$app_dir/Contents/PlugIns/platforms/libqcocoa.dylib" \
    -executable="$app_dir/Contents/PlugIns/platforms/libqoffscreen.dylib"
# Ad-hoc signing is required on Apple Silicon; it is not a Developer ID signature.
codesign --force --deep --sign - "$app_dir"
plutil -lint "$app_dir/Contents/Info.plist"
codesign --verify --deep --strict "$app_dir"
echo "Created $app_dir"