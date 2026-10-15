# Localization strings for LMN Flash (en-US).
app-title = LMN Flash

# "About" dialog (the round "i" button on the Firmware Lookup page).
about-title = About
about-version = Version
about-intro = LMNFlash wouldn't be a reality without communities' contribution.
about-thanks-heading = Specially thanks to following developers:
about-thanks-list =
    AlphaEva, for Tiny Fastboot Script and Firmware Lookup feature
    Ethan, for providing macOS build Notarization
    Littlenine Ennea, for exploit discoveries
    MlgmXyysd, for Lenovo Tablet Bootloader Unlock File generator on
about-android-heading = You can also get Android app of LMN Flash from here:
about-link-lenovobl = https://lenovobl.neko.ink/
about-link-android = https://github.com/HikariCalyx/lmnflash_android/releases/latest
about-source-code = The source code of LMNFlash PC can be found here:
about-link-source = https://github.com/HikariCalyx/lmnflash

mode-1 = Firmware Lookup
mode-2 = Smart Device Flashing
mode-3 = Firmware Decrypt

flash-bootloader-title = Bootloader Unlock
flash-bootloader-smartphone-button = Smartphone
flash-bootloader-tablet-button = Tablet

flash-bootloader-choose = How do you want to unlock the bootloader?
flash-bootloader-guided = Guided
flash-bootloader-manual = Manual

guided-title = Bootloader Unlock (Guided)
guided-intro = This guided mode walks you through unlocking your phone's bootloader with a Motorola account. You will sign in, read the Device ID, request the Unlock Key by email, and finally unlock the phone.
guided-click-login = Press the button below to open a window where you can sign in to your Motorola account.
guided-login = Login
guided-change-account = Change Account
guided-opening = Opening the sign-in window…
guided-signing-out = Signing out…
guided-getting-account = Getting account info…
guided-logged-in-as = Logged in as { $name } with Email address: { $email }
guided-next = Next Step >
guided-login-dialog-title = Motorola Bootloader Unlock — Sign in
guided-login-required = Sign in to your Motorola account first.
guided-read-desc = Read the unique Device ID from the phone (fastboot mode). This is the number Motorola uses to issue your unlock key.
guided-request-unlock-key = Request Unlock Key
guided-request-hint = Motorola sends the Unlock Key to the email address of your Motorola account. Make sure your phone is connected in fastboot mode before requesting.
guided-request-heading = Request Unlock Key
guided-request-pending = Choose below whether to proceed with the request.
guided-request-confirm = Requesting an Unlock Key will mark your phone as warranty void on Motorola's customer service system. Proceed anyway?
guided-yes = Yes
guided-no = No
guided-requesting = Requesting the unlock key from Motorola… This can take a moment.
guided-try-again = Try Again
guided-email-check = Please check the inbox of your email address { $email } — an email with the subject "Unlocking Your Device" should arrive with your Unlock Key.
guided-key-desc = Enter the Unlock Key you received by email into the field below, then press "Unlock".
flash-bootloader-already-unlocked = The bootloader is already unlocked.
flash-bootloader-return = Return
flash-bootloader-manual-desc = To request bootloader unlock, you need to obtain the Device ID (also called Unlock Data) from the device. Click "Read from Fastboot" to read it; the obtained Device ID will be copied to the clipboard.
flash-bootloader-open-site = Head over to Motorola Bootloader Unlock Website
flash-bootloader-device-id = Device ID
flash-bootloader-copy = Copy
flash-bootloader-read = Read from Fastboot
flash-bootloader-key-desc = Once you have the unlock key, type it into the field below, then click "Unlock".
flash-bootloader-unlock = Unlock
flash-bootloader-copied = Device ID copied to the clipboard.
flash-bootloader-unlocked = Bootloader unlocked.
flash-bootloader-key-required = Enter the unlock key first.
flash-bootloader-oem-unlocking-required = Turn on "OEM unlocking" in Developer Options, then try again.
flash-bootloader-unlock-cancelled = Bootloader unlock request has been cancelled.
flash-bootloader-unlock-wrong-key = Bootloader unlock failed due to wrong unlock key.
flash-bootloader-obtain-key = You need to submit your Device ID to Motorola Bootloader Unlock Website to obtain unlock key.
flash-bootloader-warranty-note = If you're unable to visit Motorola Bootloader Unlock Website, you can send the Device ID to someone who can visit. Keep in mind, once the unlock key is obtained, your phone will be marked as warranty void on Motorola's customer service system.
flash-bootloader-key-is-device-id = Device ID is not your Unlock Key
flash-bootloader-unlocking = Unlocking bootloader…
flash-bootloader-checking = Checking bootloader unlock eligibility…
flash-bootloader-eligible = This phone is able to unlock the bootloader.
flash-bootloader-not-eligible = This phone may be unable to unlock the bootloader.

flash-factory-reset-title = Factory Reset
flash-factory-reset-button = Factory Reset
factory-reset-desc = Erase all user data from the phone (factory data reset). The phone must be connected in fastboot mode (bootloader). This permanently deletes all data and cannot be undone.
factory-reset-no-frp = Note: Doing factory reset here will not erase Factory Reset Protection triggered by your Google Account.
factory-reset-select-device = Select a device
factory-reset-reading-devices = Reading devices…
factory-reset-checking = Checking the device requirements…
factory-reset-confirm = This will erase all user data from the phone (factory data reset) and cannot be undone. Proceed?
factory-reset-resetting = Performing the factory reset… Do not disconnect the phone.
factory-reset-done = Factory reset completed.
factory-reset-failed = Factory reset failed
factory-reset-blocked-locked = The bootloader is relocked. Please unlock the bootloader, or try to do it under recovery mode.
factory-reset-blocked-fdr = The bootloader does not allow a factory data reset. Please turn off "Device Reset Protection" in Settings - Security - More security settings.
factory-reset-refresh = Refresh

flash-firmware-title = Firmware Flash
flash-firmware-smartphone-button = Smartphone
flash-firmware-tablet-button = Tablet
flash-multi-flashing-title = Multi Flashing
firmware-flash-desc = Flash a complete firmware package to a phone connected in fastboot mode. Select the firmware ZIP, or the flashfile.xml together with its image files. This overwrites the firmware and erases all data on the phone.
firmware-flash-desc-no-erase = Flash a complete firmware package to a phone connected in fastboot mode. Select the firmware ZIP, or the flashfile.xml together with its image files. This overwrites the firmware on the phone.
firmware-flash-select = Select firmware package
firmware-flash-no-package = No package selected
firmware-flash-loading = Reading the firmware package…
firmware-flash-tool = Flashing tool
firmware-flash-tool-builtin = Built-in fastboot
firmware-flash-tool-platform-tools = Google Platform Tools
firmware-flash-tools-installing = Downloading Google Platform Tools…
firmware-flash-tools-failed = Could not download Google Platform Tools: { $error }
firmware-flash-tools-ready = Google Platform Tools are ready.
firmware-flash-tool-version = Reported version: { $version }
firmware-flash-tool-version-reading = Asking the tool for its version…
firmware-flash-tool-version-failed = Could not ask for the version: { $error }
firmware-flash-tools-update = Update
firmware-flash-terminal = Open Terminal
firmware-flash-terminal-preparing = Preparing the terminal…
firmware-flash-terminal-opened = Terminal opened at { $path }.
firmware-flash-terminal-failed = Could not open the terminal: { $error }
firmware-flash-terminal-minimal-adb = Minimal ADB and Fastboot is installed on this PC. Uninstall it before using this terminal: its adb and fastboot would be used instead of the tools downloaded here.
firmware-flash-rosetta-needed = Motorola's mfastboot is an Intel application, and this Mac has no Rosetta 2 to run it with.
firmware-flash-rosetta-install = Install Rosetta 2 with "softwareupdate --install-rosetta --agree-to-license" in Terminal, then reopen this dialog. Until then, use the built-in fastboot.
firmware-flash-box64-needed = Motorola's mfastboot is an Intel application, and this ARM Linux system has no box64 to run it with.
firmware-flash-box64-install = Install box64 (github.com/ptitSeb/box64) and reopen this dialog. Until then, use the built-in fastboot.
firmware-flash-verify = Verify image checksums (MD5)
firmware-flash-model = Phone model
firmware-flash-version = Software version
firmware-flash-cid = CID
firmware-flash-project-code = Project Code
firmware-flash-cid-mismatch = This firmware package is for CID { $package }, but the connected phone reports { $device }.
firmware-flash-project-mismatch = This firmware package is for project code "{ $package }", but the connected phone is "{ $device }". Flashing it will brick the phone!
firmware-flash-brick-countdown = Flashing the wrong project code bricks the phone. "Yes" becomes clickable in { $seconds } s.
firmware-flash-device-securestate = Unlock Status
firmware-flash-device-cid = CID
firmware-flash-cid-ignored = { $cid } (ignored)
firmware-flash-device-product = Project Code
firmware-flash-device-carrier = Carrier
carrier-retail = Retail { $region }
carrier-retail-open = Open { $region }
carrier-unknown = Unknown
carrier-region-cn = China
carrier-region-gb = United Kingdom
carrier-region-apac = Asia-Pacific
carrier-region-eu = Europe
carrier-region-in = India
carrier-region-kr = Korea
carrier-region-mea = Middle East
carrier-region-ru = Russia
carrier-region-de = Germany
carrier-region-pl = Poland
carrier-region-rs = Serbia
carrier-region-is = Iceland
carrier-region-la = Latin America
carrier-region-us = United States
carrier-region-ca = Canada
carrier-region-ar = Argentina
carrier-region-br = Brazil
carrier-region-cl = Chile
carrier-region-mx = Mexico
carrier-region-pe = Peru
carrier-op-cmcc = China Mobile
carrier-op-ctcn = China Telecom
carrier-op-docomo = NTT Docomo
carrier-op-softbank = SoftBank
firmware-flash-securestate-locked = Not Unlocked
firmware-flash-securestate-unlocked = Unlocked
firmware-flash-securestate-relocked = Relocked
firmware-flash-securestate-engineering = Prototype
firmware-flash-steps = Steps
firmware-flash-ignored = Ignored partitions: { $partitions }
firmware-flash-start = Start Flashing
firmware-flash-read-info = Read Info
firmware-flash-hide-sensitive = Hide Sensitive Info
firmware-flash-show-sensitive = Show Sensitive Info
firmware-flash-export = Export to Script
firmware-flash-export-done = Script written to { $path }.
firmware-flash-export-failed = Could not write the script: { $error }
firmware-flash-export-copy-question = Would you like to copy the fastboot binary next to the script as well?
firmware-flash-export-copy-note = { $tool } is copied into the folder of the script, and the script starts that copy. On Windows the DLLs it needs are copied as well.
firmware-flash-confirm = Flashing will overwrite the firmware on the phone and erase all data. Do not disconnect the phone. Proceed?
firmware-flash-flashing = Flashing… Do not disconnect the phone.
firmware-flash-step = Step { $index }/{ $total }: { $step }
firmware-flash-log = Log
firmware-flash-locate-log = Locate log
firmware-flash-done = Flashing completed.
firmware-flash-failed = Flashing failed
firmware-flash-reboot = Reboot
firmware-flash-rebooting = Rebooting…
firmware-flash-reboot-sent = Reboot command sent.
firmware-flash-reboot-failed = Reboot failed
firmware-flash-reboot-system = Normal Reboot
firmware-flash-reboot-bootloader = Bootloader
firmware-flash-reboot-recovery = Recovery
firmware-flash-reboot-fastbootd = Fastbootd
firmware-flash-reboot-sideload = ADB Sideload
firmware-flash-reboot-switch-slot = Switch to another Boot Slot
firmware-flash-edit = Edit
firmware-flash-select-all = Select all
firmware-flash-select-part = Select { $part } ({ $count })
firmware-flash-select-none = Deselect all
firmware-flash-erase-userdata = Erase Userdata
firmware-flash-erase-nv-cache = Erase NV cache
firmware-flash-erase-list = Will also erase: { $partitions }
firmware-flash-procedures-none = Select at least one step to flash.

login-dialog-title = LMN Flash — Log in
login-prompt = Log in to look up firmware
login-button = Log in
login-button-hint = Left-click: built-in browser · Right-click: log in manually
login-fetching = Fetching login URL…
login-webview-open = Complete the login in the dialog window.
login-button-hint-browser = Left-click: log in with your browser · Right-click: log in manually
login-browser-waiting = Complete the login in your browser — this window signs in as soon as you return.
login-browser-cookie-hint = Did you get HTTP error 400 during login? Please clear Cookies under domain lenovo.com with your web browser.
login-browser-cookie-chrome = In Chrome: Privacy and security - View permissions and data stored across sites (chrome://settings/content/all), search lenovo.com and delete the shown data.
login-browser-cookie-edge = In Edge: Privacy, search and services - Site permissions - All sites (edge://settings/content/all), search lenovo.com and delete the shown data.
login-browser-cookie-firefox = In Firefox: Privacy & Security - Cookies and Site Data - Manage Data… (about:preferences#privacy), search lenovo.com and remove the shown data.
login-browser-cookie-other = Open your browser's cookie or site-data settings, search lenovo.com and delete the shown data.
login-retry-webview = Retry with WebView
login-manual-prompt = Log in with your browser, then paste the SoftwareFix://callback link:
login-manual-placeholder = SoftwareFix://callback?Authorization=…
login-open-browser = Open in browser
login-copy-url = Copy URL
login-url-label = Login URL
login-submit = Confirm
login-cancel = Cancel
login-back = Back
login-error = Login failed: { $error }
login-webview-fallback = The built-in login window could not be opened; please log in with your browser instead.
login-protocol-title = softwarefix:// link handler
login-protocol-none = No program is currently registered to open softwarefix:// links.
login-protocol-current = softwarefix:// links currently open with { $program }.
login-protocol-ours = LMN Flash currently handles softwarefix:// links.
login-protocol-switch = Use LMN Flash
login-protocol-restore = Restore the original handler
login-protocol-failed = Could not change the softwarefix:// handler: { $error }

lookup-mode-row = Lookup by IMEI
lookup-mode-retcn = Lookup by device info
lookup-mode-tablet = Tablets
lookup-mode-by-model = Lookup by Model
by-model-error-required = Model name is required
category-phone = Phone
category-tablet = Tablet
category-smart = Smart
by-model-category-label = Device type
by-model-country-label = Country code
lookup-imei-label = IMEI
lookup-mode-bulk-imei = Bulk Lookup by IMEI
bulk-imei-label = IMEIs (one per line)
bulk-imei-placeholder = One 15-digit IMEI per line
bulk-error-empty = Enter at least one IMEI
bulk-progress = Looking up { $current } of { $total }…
bulk-save = Save CSV…
bulk-saved = Saved to { $path }
bulk-save-failed = Could not save the file: { $error }
bulk-summary = { $total } IMEIs looked up: { $found } found, { $invalid } invalid, { $failed } failed
lookup-imei-placeholder = 15-digit IMEI
lookup-button = Look up
lookup-fetching = Looking up firmware…
imei-error-digits = IMEI must contain digits only
imei-error-length = IMEI must be 14 or 15 digits
imei-error-checksum = IMEI checksum is invalid

tablet-sn-label = Serial number
tablet-error-sn = Serial number is required
tablet-copy-password = Copy extraction password
cn-product-name = Product name
cn-product-model = Product model
cn-market-name = Market name
cn-mtm-compat = Compatible MTM
cn-latest-version = Latest version
cn-id = Resource ID

retcn-sn-label = Serial number
retcn-model-label = XT code
retcn-fingerprint-label = Build fingerprint
retcn-carrier-label = Carrier
retcn-platform-label = Platform
retcn-fsg-label = FSG version
retcn-sim-label = SIM slots
platform-qualcomm = Qualcomm
platform-mediatek = MediaTek or Unisoc
sim-single = Single SIM
sim-dual = Dual SIM
retcn-error-model = XT code is required
retcn-error-fingerprint = Build fingerprint is required
retcn-error-carrier = Carrier is required
retcn-error-sn = Serial number is required
retcn-error-fsg = FSG version is required for Qualcomm
retcn-fill-fastboot = Fill from Fastboot
retcn-fill-fastboot-fetching = Reading device…
retcn-fill-fastboot-filled = Filled from device { $serial }
retcn-fill-fastboot-no-device = No fastboot device connected
retcn-fill-fastboot-unsupported-device = No supported Motorola device found
retcn-pick-device-title = Select a fastboot device
fw-market-name = Market name
fw-model-name = Model name
fw-sale-model = Sale model
fw-carrier = Carrier
fw-publish-date = Publish date
fw-file-name = File name
fw-file-size = File size
fw-rom-id = ROM resource ID
fw-rom-match-id = ROM match ID
fw-fingerprint = Fingerprint
fw-comments = Comments
fw-copy-uri = Copy Download URI
fw-copy-tool = Copy Tool URI
fw-copy-raw = Copy raw response
lang-en = English
lang-zh = 简体中文
lang-zhhant = 繁體中文
lang-ja = 日本語
lang-ko = 한국어
lang-ru = Русский
lang-da = Dansk
lang-de = Deutsch
lang-fr = Français
lang-it = Italiano
lang-nb = Norsk
lang-nl = Nederlands
lang-ptbr = Português (Brasil)
lang-fi = Suomi
lang-es = Español
lang-sv = Svenska
lang-uk = Українська
debug-info = Debug info
debug-account = Account
debug-token = Bearer token
debug-uuid = Client UUID

decrypt-select-dir = Select directory
decrypt-title = Tablet Firmware Decrypt
decrypt-description = This feature will decrypt t and x files used in L-brand Tablet Firmware.
decrypt-no-dir = No directory selected
decrypt-custom-password = Use custom password
decrypt-password-label = Password
decrypt-button = Decrypt
decrypt-working = Decrypting firmware files…
decrypt-done = Decryption finished: { $ok } succeeded, { $fail } failed
decrypt-no-files = No *.x or *.t files found in the selected directory
decrypt-password-required = Enter a decryption password, or uncheck "Use custom password"
decrypt-failed-files = Failed files

flash-driver-title = Install Driver
driver-smartphone-button = Smartphone
driver-tablet-button = Tablet
driver-install-button = Install
driver-desc = Motorola's Mobile Drivers are installed on this computer. Press Install to download the setup program and start it.
driver-linux-desc = The Android udev rules are installed into /etc/udev/rules.d and udev is restarted, so that adb and fastboot can access phones as a normal user. This needs administrator rights.
driver-downloading = Downloading the driver…
driver-installing = Running the installer…
driver-installing-linux = Installing the udev rules…
driver-done = The setup program has been started. Follow its instructions to finish the installation.
driver-done-linux = The udev rules have been installed.
driver-password = Password
driver-failed = Could not install the driver: { $error }
driver-error-password = The password was not accepted. Please try again.
driver-error-no-sudo = sudo was not found, so the udev rules cannot be installed.

# "Remove System Bloatware" dialog (Mode 2).
flash-bloatware-title = Remove System Bloatware
flash-bloatware-button = Remove Bloatware
bloatware-desc = Remove preinstalled apps from a device connected with USB debugging enabled. The apps are removed for the current user only; the system partition is left untouched.
bloatware-preparing = Preparing ADB and looking for devices…
bloatware-refresh = Refresh
bloatware-select-device = Select a device
bloatware-no-device = No device with USB debugging was found. Enable USB debugging in Developer Options, connect the device, and allow this computer when it asks.
bloatware-unauthorized = The device has not authorized this computer yet. Enable USB debugging, then accept the "Allow USB debugging?" prompt on the device.
bloatware-retry = Retry
bloatware-checking = Checking the installed apps…
bloatware-found-heading = Select the apps to remove:
bloatware-search = Search apps…
bloatware-none-found = None of the known apps are installed on this device.
bloatware-no-match = No app matches "{ $query }".
bloatware-select-all = Select all
bloatware-select-none = Deselect all
bloatware-remove = Remove Selected
bloatware-remove-none = Select at least one app to remove.
bloatware-removing = Removing the selected apps… Do not disconnect the device.
bloatware-done = Removal finished.
bloatware-result-removed = { $name } was removed.
bloatware-result-failed = { $name }: { $error }
bloatware-undo-note = A removed app can be restored with "pm install-existing --user 0 <package>" over ADB until the device is factory reset.
bloatware-app-browser = Browser
bloatware-app-tianxi-agent = Tianxi Agent
bloatware-app-push-service = Push Service
bloatware-app-xiaotian-trigger = Xiaotian Voice Trigger
bloatware-app-moto-ai-eraser = MotoAIEraser
bloatware-app-ai-search = AI Search
bloatware-app-smart-scenarios = Smart Scenarios
bloatware-app-ai-travel = AI Travel
bloatware-app-moto-iot-service = Moto Iot Service
bloatware-app-lock-screen-carousel = Lock screen carousel
bloatware-app-smartfeed = SmartFeed
bloatware-app-lenovo-app-store = Lenovo App Store
bloatware-app-quickapp = QuickApp
# Names of the apps whose name is translated; the carrier apps carry their
# product name in the table itself, like the carrier brands in `src/carrier.rs`.
bloatware-app-tomato-novel = Tomato Free Novel
bloatware-app-toutiao = Today's Headline
bloatware-app-douyin = TikTok CN
bloatware-app-douyin-lite = TikTok Rapid CN
bloatware-app-tiktok-global = TikTok Global
bloatware-app-tiktok-lite-global = TikTok Lite Global
bloatware-app-kuaishou = Kuaishou
bloatware-app-kuaishou-lite = Kuaishou Rapid
bloatware-app-baidu-search = Baidu Search
bloatware-app-weibo = Weibo
bloatware-app-zuimei-weather = Zui Mei Weather
bloatware-app-quickapp-honor = QuickApp (Honor)
bloatware-app-quickapp-huawei = QuickApp (Huawei)
bloatware-app-quickapp-meizu = QuickApp (Meizu)
bloatware-app-quickapp-xiaomi = QuickApp (Xiaomi)
bloatware-app-quickapp-oppo = QuickApp (OPPO)
bloatware-app-quickapp-nubia = QuickApp (Nubia)
bloatware-app-quickapp-vivo = QuickApp (vivo)
bloatware-app-quickapp-zte = QuickApp (ZTE)
bloatware-app-rednote = RedNote
bloatware-app-baidu-map = Baidu Map
bloatware-app-hongguo = Hongguo Short Drama
bloatware-app-att-iqi = AT&T Carrier IQ
bloatware-app-att-device-help = AT&T Device Help
bloatware-app-att-vvm = AT&T Visual Voicemail
bloatware-app-att-cloud = AT&T Personal Cloud
bloatware-app-att-discovery = AT&T Discovery Hub
bloatware-app-temu-global = Temu Global
bloatware-app-pinduoduo = Temu CN
bloatware-app-honor-of-kings-cn = Honor of Kings CN
bloatware-app-honor-of-kings-global = Honor of Kings Global
bloatware-app-arena-of-valor-eu = Arena of Valor EU
bloatware-app-arena-of-valor-sea = Arena of Valor SEA
bloatware-app-arena-of-valor-jp = Arena of Valor JP
bloatware-app-genshin-impact-cn = Genshin Impact CN
bloatware-app-genshin-impact-global = Genshin Impact Global
bloatware-app-genshin-impact-cloud = Genshin Impact Cloud
bloatware-app-genshin-impact-tencent = Genshin Impact Tencent
bloatware-app-delta-force-cn = Delta Force CN
bloatware-app-delta-force-global = Delta Force Global
bloatware-app-delta-force-sea = Delta Force SEA
bloatware-app-pubg-mobile-cn = PUBG Mobile CN
bloatware-app-pubg-mobile-lite = PUBG Mobile Lite
bloatware-app-pubg-mobile-kr = PUBG Mobile KR
bloatware-app-pubg-mobile-vn = PUBG Mobile VN
bloatware-app-pubg-mobile-in = PUBG Mobile IN
bloatware-app-honkai-starrail-cn = Honkai Starrail CN
bloatware-app-honkai-starrail-global = Honkai Starrail Global
bloatware-app-onmyoji-cn = Onmyoji CN
bloatware-app-onmyoji-na = Onmyoji NA
bloatware-app-onmyoji-gb = Onmyoji GB
bloatware-app-onmyoji-hmt = Onmyoji HK/MO/TW
bloatware-app-onmyoji-tencent = Onmyoji Tencent
bloatware-app-valorant-mobile = Valorant Mobile
bloatware-app-wild-rift-global = League of Legends Mobile Global
bloatware-app-lol-cn = League of Legends CN

flash-bootloader-tablet-desc = To request bootloader unlock, you need to get the tablet's serial number, as well as the Bootloader_SN (if applicable).
flash-bootloader-tablet-site-desc = Once you get them, you can input them to the ZUI Bootloader Unlock Website.
flash-bootloader-tablet-site-button = Head over to Lenovo ZUI Bootloader Unlock Website (Chinese only)
flash-bootloader-tablet-serial = Serial Number
flash-bootloader-tablet-sn = Bootloader_SN
flash-bootloader-tablet-sn-na = Not applicable
flash-bootloader-tablet-legion = Please use the Legion Y700 5th Gen section to request bootloader unlock.
flash-bootloader-tablet-unlock-desc = Once you have successfully requested bootloader unlock, you can click the following button to get the bootloader unlock key.
flash-bootloader-tablet-not-generated = The Unlock Code for your device is not generated yet. Please generate it first, or try again later if you already did.
flash-bootloader-tablet-confirm-unlock = Please take a look at your tablet to confirm bootloader unlock.
flash-bootloader-tablet-unknown-error = An unknown error has occurred. { $error }
flash-bootloader-tablet-guided-connect = Connect the tablet in fastboot mode, then select it below.
flash-bootloader-tablet-guided-working = Attempting bootloader unlock…
flash-bootloader-tablet-guided-failed = Automatic unlock failed: { $error }
flash-bootloader-tablet-guided-manual = Automatic unlock did not work. Please continue with manual unlock.
flash-bootloader-tablet-confirm-mediatek = Please press the Volume Up button to confirm the bootloader unlock.
flash-bootloader-tablet-confirm-qualcomm = Please use the volume key to select "UNLOCK THE BOOTLOADER", power key to confirm.
flash-bootloader-tablet-cancel-restart = Cancel & Restart
flash-bootloader-tablet-cancelling = Cancelling…

# "Unpack Image" dialog (Mode 2).
unpack-title = Unpack Image
unpack-button = Unpack
unpack-description = Extract the files stored in a M-branded Smartphone firmware image whose header magic is SINGLE_N_LONELY, such as radio.img.
unpack-select-image = Select image…
unpack-select-folder = Select output folder…
unpack-none-selected = Not selected
unpack-contents = Contents ({ $count } files, { $size })
unpack-start = Unpack
unpack-listing = Reading the image…
unpack-not-an-image = This file is not a M-branded Smartphone image (the SINGLE_N_LONELY header is missing).
unpack-working = Unpacking…
unpack-done = Unpacked { $count } files ({ $size }).
unpack-locate = Locate
unpack-failed = Could not unpack the image: { $error }

# "Unisoc PAC" package tool (Mode 3).
pac-title = Unisoc PAC
pac-unpack-button = Unpack
pac-repack-button = Repack
pac-description = Extract the files stored in a Unisoc (Spreadtrum) PAC firmware package, and write a flash.cfg describing it alongside them.
pac-select-file = Select PAC file…
pac-listing = Reading the package…
pac-invalid = This file is not a Unisoc PAC package (the PAC header is missing).
pac-encrypted = This PAC package is encrypted, so its files cannot be extracted.
pac-failed = Could not unpack the package: { $error }
pac-select-all = Select all
pac-select-none = Deselect all
pac-none-chosen = Select at least one file to unpack.
pac-search = Search files…
pac-no-match = No file matches "{ $query }".
pac-partial = The package is incomplete ({ $found } of { $expected }), so some of its files are missing or cut short.
pac-incomplete = incomplete
# A script names the two download agents in its command, so both have to be part
# of what the run writes before the option can be asked for.
pac-spd-need-fdl = Select both download agents (FDL and FDL2) to generate the scripts.
# Also write the two `spd_dump` scripts next to the files: one flashes the whole
# firmware over USB from the scattered files, the other erases the data that
# belongs to the phone's owner. They describe what is written out, so they need
# the package's own configuration XML.
pac-spd-script = Generate SPD Dump Script
pac-spd-written = Scripts written: { $files }
pac-spd-failed = Could not write the scripts: { $error }
pac-spd-no-config = This package carries no configuration XML, so the partition names are not known.

# "Repack Package" dialog (Mode 3): builds a PAC package from a flash.cfg or a
# scatter XML and the files next to it.
pac-repack-title = Repack Package
pac-repack-description = Build a Unisoc PAC package from a flash.cfg or a scatter XML and the files next to it: every record of the source is matched to the file it needs.
pac-repack-select-source = Select flash.cfg or scatter XML…
pac-repack-reading = Reading the source…
pac-repack-invalid = This file is neither a flash.cfg nor a scatter XML.
pac-repack-files = { $resolved } of { $total } files found
pac-repack-needs-file = no file yet
pac-repack-select-file = Choose file
pac-repack-leave-out = Leave out the { $count } without a file
pac-repack-alias = Product alias
pac-repack-version = Product version
pac-repack-metadata = A scatter XML does not name the version of the product, so fill in what the package should carry.
pac-repack-nothing = Select at least one file to write.
pac-repack-save = Save package as…
pac-repack-working = Writing the package…
pac-repack-done = Wrote { $count } files ({ $size }) to { $path }.
pac-repack-failed = Could not repack: { $error }

# The record table's columns.
pac-repack-col-part = Part Name
pac-repack-col-partition = Actual Partition Name
pac-repack-col-size = Size
pac-repack-col-file = FileName

# "Multi Flashing" page (Mode 2): entered from the Firmware Flash dialog.
flash-multi-flashing-button = Open
multi-flash-idle = No device is being flashed yet.
multi-flash-exit = Exit
multi-flash-preset-manage = Preset Manage
multi-flash-preset-add = Add
multi-flash-preset-finish = Finish
multi-flash-preset-empty = No firmware image loaded yet.
multi-flash-start = Start
multi-flash-stop = Stop
multi-flash-close-prompt = Please Stop the operation before closing.
multi-flash-close-prompt-ok = OK

# The run itself: the table of the devices the session finds, and what it says
# about each of them. "Logs are written to" names the directory of this run, so
# the user can read what happened to a phone afterwards.
multi-flash-col-progress = Progress
multi-flash-col-current = Current Progress
multi-flash-no-preset = No available presets for this device
multi-flash-device-finished = Finished
multi-flash-device-failed = Failed: { $error }
multi-flash-run-failed = Could not start the run: { $error }
multi-flash-still-flashing = { $count } devices are still being flashed. Do not close the window until they are done.
multi-flash-log-directory = Logs are written to { $path }

# Firmware Flash: the selected ZIP is not a firmware package.
firmware-flash-invalid-package = The selected ZIP archive is not a valid firmware package (no flashfile.xml inside)

# Multi Flashing: the options a preset is set up with.
multi-flash-options-title = Firmware Options
multi-flash-options-reboot = Reboot the device when this firmware is done
multi-flash-options-conditions = Flash conditions
multi-flash-options-default = Default
multi-flash-options-default-desc = Flash when the device CID matches this firmware. An engineering bootloader reports 0xDEAD instead of a real CID, and matches as well.
# A preset whose default rule is told not to take an engineering unit: such a
# bootloader reports the placeholder CID 0xDEAD, so it would otherwise be taken
# whatever the firmware is for.
multi-flash-options-no-engineering = Do not match engineering unit
multi-flash-options-custom = Custom
multi-flash-options-cid = CID
multi-flash-options-cid-add = Add
# The dropdown beside the CID field: the codes the community documents, which
# fill the field instead of being typed. Its entries are data (see src/cid.rs).
multi-flash-options-cid-preset = Preset
# The names of the CIDs the dropdown offers. A carrier's name ("Verizon") reads
# the same everywhere and is data, not a message — only what a code is *for* is
# translated here (see src/cid.rs).
cid-name-retcn = China retail
cid-name-global = Global
cid-name-usa-lockdown = USA Lockdown
cid-name-super = SuperCID
cid-name-factory = FactoryCID
cid-name-invalid = Invalid
cid-name-unknown = Unknown
cid-name-korea-sk-kt = Korea SK/KT
cid-name-europe-apac-retail = Europe/Asia-Pacific retail
cid-name-us-retail = US retail
cid-name-latam-retail = Latin America retail
cid-name-unknown-deprecated = Unknown (deprecated)
cid-name-canada-retail = Canada retail
cid-name-google-developer = Google Developer Edition
multi-flash-options-states = Bootloader unlock state
multi-flash-options-incomplete = Add at least one CID and pick at least one unlock state.
multi-flash-options-confirm = Confirm
# Where a ZIP package is unpacked, since a factory package is gigabytes of
# images: the user picks the directory, the firmware is checked first, and
# only then is anything written into it.
multi-flash-options-extract = Extract to
multi-flash-options-extract-choose = Choose…
multi-flash-options-extract-required = Choose a folder to unpack this firmware into.
multi-flash-options-unpacking = Unpacking the firmware…
multi-flash-preset-reboot = Reboot when done
multi-flash-preset-edit = Edit
multi-flash-preset-remove = Remove
multi-flash-preset-codename = Codename

# Saving and loading the preset list: the file itself is JSON, so the two
# buttons only write and read it.
multi-flash-preset-save = Save
multi-flash-preset-load = Load
multi-flash-preset-saved = Presets saved to { $path }.
multi-flash-preset-loaded = Loaded { $count } presets from { $path }.
# An entry whose firmware package is gone (or names something an older release
# does not know) is left out, and the user has to see which ones.
multi-flash-preset-load-skipped = Could not load { $count } entries:
multi-flash-preset-invalid = This file is not a preset file saved by LMN Flash.
multi-flash-preset-newer = This preset file was written by a newer version of LMN Flash (version { $version }).
multi-flash-preset-failed = Could not read or write the preset file: { $error }
# Shown in the preset list while the MD5 of every image is not checked.
multi-flash-preset-no-verify = MD5 check skipped
# Two presets that would both be flashed to the same phone: a run takes the
# first one that matches, so which firmware a device gets would be decided by
# the order of the list. The list names the presets it is about, and Finish is
# shut until it is resolved.
multi-flash-preset-conflict = { $count } presets would flash the same phone: { $presets }

# "Connect ADB Wireless Debugging" dialog (Mode 2).
flash-wireless-title = Connect ADB Wireless Debugging
flash-wireless-button = Connect
wireless-desc = Connect a phone over Wi-Fi with Android's wireless debugging. Pair it with this computer first, by scanning a QR code or by entering the pairing code the phone shows. Both devices have to be on the same network.
wireless-method-qr = QR code
wireless-method-code = Pairing code
wireless-qr-desc = On the phone, open Developer options, then Wireless debugging, and tap "Pair device with QR code". Then scan the code below.
wireless-qr-regenerate = New QR Code
wireless-qr-waiting = Waiting for the phone to scan the QR code…
wireless-code-desc = On the phone, open Developer options, then Wireless debugging, and tap "Pair device with pairing code". Then pick the phone below and enter the six-digit code it shows.
wireless-code-device = Phone
wireless-code-placeholder = 6-digit pairing code
wireless-no-service = No phone is offering to pair. Keep this dialog open and start the pairing on the phone.
wireless-pair = Pair
wireless-code-required = Enter the pairing code the phone shows first.
wireless-working = Pairing with the phone…
wireless-connected = Connected to { $device }.
wireless-paired = The phone is paired. It will connect once wireless debugging is reachable.
wireless-failed = Pairing failed: { $error }
wireless-mdns-failed = Could not search the network for the phone: { $error }
wireless-disconnect = Disconnect
wireless-disconnecting = Disconnecting from the phone…
wireless-disconnected = Disconnected from { $device }.
wireless-disconnect-failed = Could not disconnect: { $error }
wireless-disconnect-all = Everything
wireless-disconnected-all = Disconnected from every wireless device.

# "Fix Captive Portal Server" dialog (Mode 2).
flash-captive-portal-title = Fix Captive Portal Server
flash-captive-portal-button = Fix Now
captive-portal-desc = Point the device's captive portal check and NTP server at servers that answer, for phones that report "Connected, no Internet". The device has to run Android 5.0 or newer.
captive-portal-note = The fix runs on the device itself. On Android 10 and newer the device also cycles airplane mode to pick the new settings up, so a wireless debugging connection is dropped while it runs. A USB connection is not affected.
captive-portal-fix = Fix
captive-portal-working = Applying the captive portal fix…
captive-portal-done = The captive portal server and the NTP server were updated.
captive-portal-disconnected = Wireless debugging to { $device } was disconnected; pair again once the device is back on the network.
captive-portal-failed = Could not apply the fix: { $error }
captive-portal-unsupported = This device runs an Android older than 5.0, which this fix does not support.
captive-portal-manual-airplane = On Android 9 and older, turn Airplane mode on and then off again on the device for the new settings to take effect.
captive-portal-retry = Retry
