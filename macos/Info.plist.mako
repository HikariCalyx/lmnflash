<%
import os
import semver
import toml

with open(os.path.join(os.path.dirname(__file__), "..", "Cargo.toml")) as f:
    cargo_toml = toml.load(f)


version = semver.Version.parse(cargo_toml["package"]["version"])
%><?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
	<dict>
		<key>CFBundleDevelopmentRegion</key>
		<string>en</string>
		<key>CFBundleExecutable</key>
		<string>lmnflash</string>
		<key>CFBundleIdentifier</key>
		<string>com.hikaricalyx.lmnflash</string>
		<key>CFBundleInfoDictionaryVersion</key>
		<string>6.0</string>
		<key>CFBundleName</key>
		<string>LMNFlash</string>
		<key>CFBundleIconFile</key>
		<string>LMNFlash.icns</string>
		<key>CFBundlePackageType</key>
		<string>APPL</string>
		<key>CFBundleShortVersionString</key>
		<string>${version.major}.${version.minor}.${version.patch}</string>
		<key>CFBundleSupportedPlatforms</key>
		<array>
			<string>MacOSX</string>
		</array>
		<!-- The scheme the official Motorola/Lenovo "Software Fix" handles and
		     whose callback `src/protocol.rs` takes over: declaring it here is
		     what lets LaunchServices offer LMN Flash for `softwarefix://`
		     links (and point the scheme at the bundle at runtime). -->
		<key>CFBundleURLTypes</key>
		<array>
			<dict>
				<key>CFBundleTypeRole</key>
				<string>Viewer</string>
				<key>CFBundleURLName</key>
				<string>com.hikaricalyx.lmnflash.softwarefix</string>
				<key>CFBundleURLSchemes</key>
				<array>
					<string>softwarefix</string>
				</array>
			</dict>
		</array>
		<key>CFBundleVersion</key>
		<string>1</string>
		<key>LSMinimumSystemVersion</key>
		<string>11.0</string>
		<key>NSHumanReadableCopyright</key>
		<string></string>
	</dict>
</plist>
