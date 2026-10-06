//! The "Remove System Bloatware" feature (Mode 2): the apps it offers to
//! remove, and the ADB commands that find and uninstall them.
//!
//! The dialog only *presents* this: the flow state and its update handlers
//! live in `crate` (`State::flash.bloatware`), and the card itself in
//! [`crate::bloatware`]. This module is the part that talks to the phone.
//!
//! The commands run through the `adb` of the Android platform-tools, which
//! are downloaded next to `fastboot` on first use (see
//! [`crate::platform_tools`]) — the app ships no ADB of its own, and using
//! whichever `adb` happens to be on the `PATH` would make the tooling
//! unpredictable. Uninstalling is done per user (`pm uninstall --user 0`), so
//! only the current user loses the app: `pm install-existing --user 0 <pkg>`
//! (or a factory reset) brings it back, and the system partition is never
//! touched.

use std::process::Stdio;

use crate::platform_tools;

/// A preinstalled app the feature offers to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bloatware {
    /// Android package name, as the device reports it.
    pub package: &'static str,
    /// What the checklist shows for the app.
    pub label: Label,
}

/// The name of an app in the checklist.
///
/// A carrier's app is a product name that reads the same in every language, so
/// it is data rather than a translation — the same reasoning as the carrier
/// brands in [`crate::carrier`]. The Lenovo/Motorola and the Chinese apps have
/// names worth translating and keep an FTL id instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// An FTL message id, rendered in the UI language.
    Message(&'static str),
    /// A name shown as it is.
    Text(&'static str),
}

/// A table row whose name is translated (an FTL id).
const fn translated(package: &'static str, name_id: &'static str) -> Bloatware {
    Bloatware {
        package,
        label: Label::Message(name_id),
    }
}

/// A table row whose name is a product name, shown as it is.
const fn product(package: &'static str, name: &'static str) -> Bloatware {
    Bloatware {
        package,
        label: Label::Text(name),
    }
}

impl Bloatware {
    /// The name to show in the language of `l10n`.
    pub fn label(&self, l10n: &crate::l10n::Bundle) -> String {
        match self.label {
            Label::Message(id) => l10n.tr(id),
            Label::Text(name) => name.to_owned(),
        }
    }
}

/// The apps the feature knows about, in the order the checklist shows them.
///
/// The carrier apps come from the *Universal Android Debloater* carriers list
/// (github.com/MuntashirAkon/android-debloat-list), which marks them as safe to
/// delete; apps whose name is only the package id there are shown by it.
pub const BLOATWARE: &[Bloatware] = &[
    translated("com.zui.browser", "bloatware-app-browser"),
    translated("com.lenovo.menu_assistant", "bloatware-app-tianxi-agent"),
    translated("com.lenovo.lsf.device", "bloatware-app-push-service"),
    translated("com.lenovo.xiaotian.trigger", "bloatware-app-xiaotian-trigger"),
    translated("com.moto.gallery.ai.eraser", "bloatware-app-moto-ai-eraser"),
    translated("com.motorola.cn.searchintelligence", "bloatware-app-ai-search"),
    translated("com.motorola.cn.smartscene", "bloatware-app-smart-scenarios"),
    translated("com.motorola.cn.smartservice", "bloatware-app-ai-travel"),
    translated("com.motorola.iotservice", "bloatware-app-moto-iot-service"),
    translated(
        "com.motorola.wallpaperphotoshuffle",
        "bloatware-app-lock-screen-carousel",
    ),
    translated("com.motorola.smartfeed", "bloatware-app-smartfeed"),
    translated("com.lenovo.leos.appstore", "bloatware-app-lenovo-app-store"),
    translated("com.lenovo.hyperengine", "bloatware-app-quickapp"),
    translated("com.dragon.read", "bloatware-app-tomato-novel"),
    translated("com.ss.android.article.news", "bloatware-app-toutiao"),
    translated("com.ss.android.ugc.aweme", "bloatware-app-douyin"),
    translated("com.ss.android.ugc.aweme.lite", "bloatware-app-douyin-lite"),
    // The international builds of the same app; each name is shared by every
    // package of that build.
    translated("com.ss.android.ugc.trill", "bloatware-app-tiktok-global"),
    translated("com.zhiliaoapp.musically", "bloatware-app-tiktok-global"),
    translated("com.zhiliaoapp.musically.go", "bloatware-app-tiktok-lite-global"),
    translated(
        "com.ss.android.ugc.tiktok.lite",
        "bloatware-app-tiktok-lite-global",
    ),
    translated("com.tiktok.lite.go", "bloatware-app-tiktok-lite-global"),
    translated("com.smile.gifmaker", "bloatware-app-kuaishou"),
    translated("com.kuaishou.nebula", "bloatware-app-kuaishou-lite"),
    translated("com.baidu.searchbox", "bloatware-app-baidu-search"),
    translated("com.sina.weibo", "bloatware-app-weibo"),
    translated("com.icoolme.android.weather", "bloatware-app-zuimei-weather"),
    translated("com.hihonor.quickengine", "bloatware-app-quickapp-honor"),
    translated("com.huawei.fastapp", "bloatware-app-quickapp-huawei"),
    translated("com.meizu.flyme.directservice", "bloatware-app-quickapp-meizu"),
    translated("com.miui.hybrid", "bloatware-app-quickapp-xiaomi"),
    translated("com.nearme.instant.platform", "bloatware-app-quickapp-oppo"),
    translated("com.nubia.quickrt", "bloatware-app-quickapp-nubia"),
    translated("com.vivo.hybrid", "bloatware-app-quickapp-vivo"),
    translated("com.zte.quickrt", "bloatware-app-quickapp-zte"),
    translated("com.xingin.xhs", "bloatware-app-rednote"),
    translated("com.baidu.BaiduMap", "bloatware-app-baidu-map"),
    // The domestic and the overseas build are two packages that ship under
    // the same name.
    translated("com.phoenix.read", "bloatware-app-hongguo"),
    translated("com.phoenix.read.oversea.gp", "bloatware-app-hongguo"),
    translated("com.att.iqi", "bloatware-app-att-iqi"),
    translated("com.att.dh", "bloatware-app-att-device-help"),
    translated("com.att.mobile.android.vvm", "bloatware-app-att-vvm"),
    translated("com.att.personalcloud", "bloatware-app-att-cloud"),
    translated("com.aura.jet.att", "bloatware-app-att-discovery"),
    product("ca.bell.wt.android.tunesappswidget", "App Widget"),
    product("com.LogiaGroup.LogiaDeck", "Mobile Services Manager"),
    product("com.Rogers.MyRogersTab", "com.Rogers.MyRogersTab"),
    product("com.aetherpal.attdh.se", "Device Help"),
    product("com.aetherpal.attdh.zte", "Device Help"),
    product("com.altice.android.myapps", "com.altice.android.myapps"),
    product("com.americanexpress.plenti", "com.americanexpress.plenti"),
    product(
        "com.android.partnerbrowsercustomizations.tmobile",
        "com.android.partnerbrowsercustomizations.tmobile",
    ),
    product("com.android.sprint.hiddenmenuapp", "HiddenMenu"),
    product(
        "com.android.wifi.resources.overlay.WifiVodafoneOverlay",
        "com.android.wifi.resources.overlay.WifiVodafoneOverlay",
    ),
    product("com.asurion.android.mobilerecovery.att", "AT&T Protect Plus"),
    product("com.asurion.android.mobilerecovery.sprint", "Sprint Protect"),
    product("com.asurion.android.mobilerecovery.sprint.vpl", "Sprint Protect"),
    product("com.asurion.android.protech.att", "AT&T ProTech"),
    product("com.asurion.android.verizon.vms", "Digital Secure"),
    product("com.asurion.home.sprint", "Sprint Complete"),
    product("com.asurion.home.sprint.vpl", "Tech Expert"),
    product("com.att.android.attsmartwifi", "AT&T Smart Wi-Fi"),
    product("com.att.callprotect", "AT&T Call Protect"),
    product("com.att.csoiam.mobilekey", "AT&T Sign in Helper"),
    product("com.att.deviceunlock", "com.att.deviceunlock"),
    product("com.att.dtv.shaderemote", "DIRECTV Remote App"),
    product("com.att.mobilesecurity", "AT&T ActiveArmor℠"),
    product("com.att.mobiletransfer", "AT&T Mobile Transfer"),
    product("com.att.myWireless", "myAT&T"),
    product("com.aura.oobe.att", "com.aura.oobe.att"),
    product("com.aura.oobe.motorola", "com.aura.oobe.motorola"),
    product("com.aura.oobe.samsung", "AppCloud"),
    product("com.aura.oobe.samsung.gl", "AppCloud"),
    product("com.aura.oobe.vodafone", "com.aura.oobe.vodafone"),
    product("com.bc360.android.service", "com.bc360.android.service"),
    product("com.bc360.control", "com.bc360.control"),
    product("com.claroColombia.contenedor", "com.claroColombia.contenedor"),
    product("com.cricketwireless.minus", "com.cricketwireless.minus"),
    product("com.customermobile.preload.vzw", "Verizon Store Demo Mode"),
    product("com.directv.promo.shade", "com.directv.promo.shade"),
    product("com.dti.amx", "com.dti.amx"),
    product("com.dti.att", "Mobile Services Manager"),
    product("com.dti.bouyguestelecom", "com.dti.bouyguestelecom"),
    product("com.dti.cricket", "com.dti.cricket"),
    product("com.dti.motorola", "com.dti.motorola"),
    product("com.dti.samsung", "com.dti.samsung"),
    product("com.dti.tim", "com.dti.tim"),
    product("com.dti.tracfone", "Mobile Services"),
    product("com.felicanetworks.mfc", "com.felicanetworks.mfc"),
    product("com.felicanetworks.mfm", "com.felicanetworks.mfm"),
    product("com.felicanetworks.mfm.main", "com.felicanetworks.mfm.main"),
    product("com.felicanetworks.mfs", "com.felicanetworks.mfs"),
    product("com.felicanetworks.mfw.a.boot", "com.felicanetworks.mfw.a.boot"),
    product("com.google.omadm.trigger", "com.google.omadm.trigger"),
    product("com.hyperlync.Sprint.CloudBinder", "Sprint Cloud Binder"),
    product("com.inmobi.installer", "com.inmobi.installer"),
    product(
        "com.ironsource.appcloud.oobe.hutchison",
        "com.ironsource.appcloud.oobe.hutchison",
    ),
    product("com.kmsjp", "com.kmsjp"),
    product("com.kuackmedia.orange", "com.kuackmedia.orange"),
    product("com.locationlabs.cni.att", "AT&T Smart Limits℠"),
    product("com.matchboxmobile.wisp", "AT&T Hot Spots"),
    product("com.motorola.att.phone.extensions", "ATT Phone Extension"),
    product("com.motorola.attvowifi", "Wi-Fi Calling"),
    product(
        "com.motorola.ltebroadcastservices_vzw",
        "com.motorola.ltebroadcastservices_vzw",
    ),
    product("com.motorola.mot5gmod", "5G Moto Mod"),
    product("com.motorola.omadm.sprint", "SprintDM"),
    product("com.motorola.omadm.usc", "com.motorola.omadm.usc"),
    product("com.motorola.omadm.vzw", "VzwDM"),
    product("com.motorola.sprintwfc", "Sprint Wifi Calling"),
    product("com.motorola.visualvoicemail", "Verizon Visual Voicemail"),
    product("com.motorola.vzw.cloudsetup", "Cloud setup"),
    product("com.motorola.vzw.loader", "com.motorola.vzw.loader"),
    product("com.motorola.vzw.mot5gmod", "5G Moto Mod"),
    product("com.motorola.vzw.pco.extensions.pcoreceiver", "PcoReceiver"),
    product("com.motorola.vzw.phone.extensions", "PhoneExtns"),
    product("com.motorola.vzw.provider", "VzwUnifiedSettingsApp"),
    product("com.motricity.verizon.ssodownloadable", "Verizon Login"),
    product("com.naviexpert.NaviExpert", "com.naviexpert.NaviExpert"),
    product("com.nextbit.app", "com.nextbit.app"),
    product("com.nim.rogers", "com.nim.rogers"),
    product(
        "com.nttdocomo.android.applicationmanager",
        "com.nttdocomo.android.applicationmanager",
    ),
    product(
        "com.nttdocomo.android.iconcier_contents",
        "com.nttdocomo.android.iconcier_contents",
    ),
    product(
        "com.nttdocomo.android.initialization",
        "com.nttdocomo.android.initialization",
    ),
    product(
        "com.nttdocomo.android.rwpushcontroller",
        "com.nttdocomo.android.rwpushcontroller",
    ),
    product("com.nttdocomo.android.store", "com.nttdocomo.android.store"),
    product("com.oem.euiccpartnerapp", "com.oem.euiccpartnerapp"),
    product("com.orange.aura.oobe", "Orange Manual Selector"),
    product("com.orange.miorange", "Mi Orange"),
    product("com.orange.mylivebox.fr", "Ma Livebox"),
    product("com.orange.mysosh", "MySosh France"),
    product("com.orange.orangeetmoi", "Orange et moi France"),
    product("com.orange.owtv", "TV d'Orange"),
    product("com.orange.tdd", "Transfert des données"),
    product("com.orange.update", "App Center"),
    product(
        "com.orange.update.OrangeUpdateApplication",
        "com.orange.update.OrangeUpdateApplication",
    ),
    product("com.orange.vvm", "Messagerie vocale visuelle"),
    product("com.orange.wifiorange", "Mon Réseau"),
    product("com.ptc.osp.gnc", "com.ptc.osp.gnc"),
    product("com.samsung.attvvm", "Samsung AT&T Visual Voicemail"),
    product("com.samsung.huxextension", "com.samsung.huxextension"),
    product("com.samsung.sprint.chameleon", "com.samsung.sprint.chameleon"),
    product("com.sec.android.app.ewidgetatt", "Entertainment Widget"),
    product("com.sec.sprint.wfcstub", "com.sec.sprint.wfcstub"),
    product("com.securityandprivacy.android.verizon.vms", "Digital Secure"),
    product("com.sfr.android.moncompte", "SFR & Moi"),
    product("com.sfr.android.sfrjeux", "com.sfr.android.sfrjeux"),
    product("com.sfr.android.sfrplay", "SFR Play"),
    product("com.sfr.android.vvm", "SFR Répondeur +"),
    product("com.sprint.care", "My Sprint"),
    product("com.sprint.ce.updater", "Mobile Installer (ソフトバンク)"),
    product("com.sprint.ecid", "Caller ID"),
    product("com.sprint.fng", "Sprint Spot"),
    product("com.sprint.international.message", "Sprint Worldwide"),
    product("com.sprint.ms.cdm", "Carrier Device Manager"),
    product("com.sprint.ms.cnap", "Caller ID"),
    product("com.sprint.ms.smf.services", "Carrier Hub"),
    product("com.sprint.psdg.sw", "Carrier Setup Wizard"),
    product("com.sprint.safefound", "Safe & Found"),
    product("com.sprint.safefound.vpl", "Safe & Found"),
    product("com.sprint.topup", "Sprint World Top-Up"),
    product("com.sprint.w.installer", "Mobile ID"),
    product("com.sprint.w.v8", "Featured Apps"),
    product("com.sprint.zone", "Sprint Zone"),
    product("com.synchronoss.dcs.att.r2g", "AT&T Ready2Go"),
    product("com.telcel.contenedor", "com.telcel.contenedor"),
    product("com.telecomsys.directedsms.android.SCG", "Verizon Location Agent"),
    product("com.telus.checkup", "com.telus.checkup"),
    product("com.telus.myaccount", "My TELUS"),
    product("com.tmobile.pr.adapt", "T-Mobile"),
    product("com.tmobile.pr.mytmobile", "T-Mobile"),
    product("com.tmobile.services.nameid", "T-Mobile Scam Shield"),
    product("com.tmobile.simlock", "Device Unlock"),
    product("com.tmobile.vvm.application", "T-Mobile Visual Voicemail"),
    product(
        "com.tracfone.preload.accountservices",
        "com.tracfone.preload.accountservices",
    ),
    product("com.verizon.cloudsetupwizard", "com.verizon.cloudsetupwizard"),
    product("com.verizon.llkagent", "Llkagent"),
    product("com.verizon.loginengine.unbranded", "Carrier Login Engine"),
    product("com.verizon.messaging.vzmsgs", "Verizon Messages"),
    product("com.verizon.mips.services", "My Verizon Services"),
    product("com.verizon.obdm_permissions", "OBDM_Permissions"),
    product(
        "com.verizon.permissions.appdirectedsms",
        "com.verizon.permissions.appdirectedsms",
    ),
    product(
        "com.verizon.permissions.vzwappapn",
        "com.verizon.permissions.vzwappapn",
    ),
    product("com.verizon.remoteSimlock", "com.verizon.remoteSimlock"),
    product("com.verizon.vzwavs", "VzwAVS"),
    product("com.verizontelematics.verizonhum", "Hum: GPS Locator"),
    product("com.vznavigator.Generic", "VZ Navigator"),
    product("com.vzw.apnservice", "VZWAPN"),
    product("com.vzw.ecid", "Verizon Call Filter"),
    product("com.vzw.hss.myverizon", "My Verizon"),
    product("com.vzw.hss.widgets.infozone.large", "My InfoZone™ Widget:Big Screen"),
    product("com.vzw.qualitydatalog", "com.vzw.qualitydatalog"),
    product("com.wavemarket.waplauncher", "AT&T Secure Family™"),
    product("com.whitepages.nameid.tmobile", "T-Mobile Name ID"),
    product("de.telekom.tsc", "AppEnabler"),
    product("fr.bouyguestelecom.ecm.android", "Bouygues Telecom"),
    product("fr.bouyguestelecom.tv.android", "B.tv"),
    product("fr.bouyguestelecom.vvmandroid", "Messagerie vocale visuelle"),
    product("fr.orange.cineday", "Orange Cineday"),
    product("hdopen.vivicitta", "hdopen.vivicitta"),
    product("hu.telekom.telekomapp", "hu.telekom.telekomapp"),
    product("hu.telekom.telekomtv", "hu.telekom.telekomtv"),
    product(
        "jp.co.daj.consumer.ifilter.aflauncher",
        "jp.co.daj.consumer.ifilter.aflauncher",
    ),
    product("jp.co.omronsoft.iwnnime.ml", "jp.co.omronsoft.iwnnime.ml"),
    product(
        "jp.co.omronsoft.wnnext.skin.std_dark_type2_HW",
        "jp.co.omronsoft.wnnext.skin.std_dark_type2_HW",
    ),
    product(
        "jp.co.omronsoft.wnnext.skin.std_light_type2_HW",
        "jp.co.omronsoft.wnnext.skin.std_light_type2_HW",
    ),
    product(
        "jp.co.yahoo.android.ebookjapan.preinstall",
        "jp.co.yahoo.android.ebookjapan.preinstall",
    ),
    product("net.aetherpal.device", "AT&T Remote Support"),
    product("pl.tmobile.miboa", "pl.tmobile.miboa"),
    product("pl.tmobile.panel", "pl.tmobile.panel"),
    product("ro.cosmote.aps.wnwlite", "ro.cosmote.aps.wnwlite"),
    product("telekom.hu.android.mobilvasarlas", "telekom.hu.android.mobilvasarlas"),
    product("tmobile.hu.android.epgmiab", "tmobile.hu.android.epgmiab"),
    product("uk.co.ee.myee", "My EE"),
    product("us.com.dt.iq.appsource.tmobile", "App Source"),
    // Games that OEMs and carriers bundle. Unlike the carrier apps above, each
    // of these publishes under a different title per region and language
    // (Genshin Impact / 原神), so their names are translated like the system
    // apps.
    translated("com.einnovation.temu", "bloatware-app-temu-global"),
    translated("com.xunmeng.pinduoduo", "bloatware-app-pinduoduo"),
    translated("com.tencent.tmgp.sgame", "bloatware-app-honor-of-kings-cn"),
    translated(
        "com.levelinfinite.sgameGlobal",
        "bloatware-app-honor-of-kings-global",
    ),
    translated("com.ngame.allstar.eu", "bloatware-app-arena-of-valor-eu"),
    translated("com.tencent.ngame.chty", "bloatware-app-arena-of-valor-sea"),
    translated("com.tencent.ngjp", "bloatware-app-arena-of-valor-jp"),
    translated("com.miHoYo.Yuanshen", "bloatware-app-genshin-impact-cn"),
    translated("com.miHoYo.GenshinImpact", "bloatware-app-genshin-impact-global"),
    translated(
        "com.hoyoverse.cloudgames.GenshinImpact",
        "bloatware-app-genshin-impact-cloud",
    ),
    translated("com.tencent.pcgame.ys", "bloatware-app-genshin-impact-tencent"),
    translated("com.tencent.tmgp.dfm", "bloatware-app-delta-force-cn"),
    translated("com.proxima.dfm", "bloatware-app-delta-force-global"),
    translated("com.garena.game.df", "bloatware-app-delta-force-sea"),
    translated("com.tencent.tmgp.pubgmhd", "bloatware-app-pubg-mobile-cn"),
    translated("com.tencent.iglite", "bloatware-app-pubg-mobile-lite"),
    translated("com.pubg.krmobile", "bloatware-app-pubg-mobile-kr"),
    translated("com.vng.pubgmobile", "bloatware-app-pubg-mobile-vn"),
    translated("com.pubg.imobile", "bloatware-app-pubg-mobile-in"),
    translated("com.miHoYo.hkrpg", "bloatware-app-honkai-starrail-cn"),
    translated(
        "com.HoYoverse.hkrpgoversea",
        "bloatware-app-honkai-starrail-global",
    ),
    translated("com.netease.onmyoji", "bloatware-app-onmyoji-cn"),
    translated("com.netease.onmyoji.na", "bloatware-app-onmyoji-na"),
    translated("com.netease.onmyoji.gb", "bloatware-app-onmyoji-gb"),
    translated("com.onmyoji.hmt2", "bloatware-app-onmyoji-hmt"),
    translated("com.tencent.tmgp.yys.zqb", "bloatware-app-onmyoji-tencent"),
    translated("com.tencent.tmgp.codev", "bloatware-app-valorant-mobile"),
    translated(
        "com.riotgames.league.wildrift",
        "bloatware-app-wild-rift-global",
    ),
    translated("com.tencent.lolm", "bloatware-app-lol-cn"),
];

/// What `adb devices` says a connected device is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbDeviceState {
    /// USB debugging is authorized: commands can run on the device.
    Ready,
    /// The device has not accepted this computer's debugging key yet, so it
    /// has to show the "Allow USB debugging?" prompt first.
    Unauthorized,
    /// The device is connected but not answering.
    Offline,
    /// A state this version does not know.
    Unknown,
}

/// A device `adb devices` reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: AdbDeviceState,
    /// The `model:` the `-l` listing carries, with its underscores turned
    /// into spaces (e.g. `moto g84 5G`), when the device reports one.
    pub model: Option<String>,
}

impl AdbDevice {
    /// What the device list shows for this device.
    pub fn label(&self) -> String {
        match &self.model {
            Some(model) => format!("{} ({model})", self.serial),
            None => self.serial.clone(),
        }
    }
}

/// The result of removing one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalOutcome {
    pub app: Bloatware,
    /// `None` when the app was removed; otherwise what the device answered.
    pub error: Option<String>,
}

impl RemovalOutcome {
    /// Whether the app was removed.
    pub fn removed(&self) -> bool {
        self.error.is_none()
    }
}

/// Whether the platforms-tools `adb` is on disk, downloading them when not.
///
/// The release artifacts ship no `adb`, and platform-tools downloaded before
/// this feature existed have only `fastboot`, so a missing `adb` is what
/// starts the download.
pub fn ensure_adb() -> Result<(), String> {
    if platform_tools::adb_present() {
        return Ok(());
    }

    platform_tools::install().map(|_| ())
}

/// Runs `adb` with `args` and returns its standard output.
///
/// Only the exit status decides between success and failure: `adb` writes its
/// daemon-start messages to stderr even when the command worked.
fn run_adb(args: &[&str]) -> Result<String, String> {
    let mut command = platform_tools::adb_command();
    command.args(args).stdin(Stdio::null());

    // A GUI application must not flash a console window on screen.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let program = command.get_program().to_string_lossy().to_string();

    let output = command
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !output.status.success() {
        let printed = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };

        return Err(printed.trim().to_owned());
    }

    Ok(stdout)
}

/// Lists the devices `adb devices` reports, making sure `adb` is there first.
pub fn list_devices() -> Result<Vec<AdbDevice>, String> {
    ensure_adb()?;

    Ok(parse_devices(&run_adb(&["devices", "-l"])?))
}

/// The bloatware packages installed on `serial`.
pub fn list_installed(serial: &str) -> Result<Vec<Bloatware>, String> {
    let output = run_adb(&["-s", serial, "shell", "pm", "list", "packages"])?;

    Ok(installed(&parse_packages(&output)))
}

/// Removes `apps` from user 0 of `serial`, one by one.
///
/// Every app is reported separately: one that refuses to go (a protected
/// system app, a device that answers with an error) must not stop the rest.
pub fn remove(serial: &str, apps: &[Bloatware]) -> Vec<RemovalOutcome> {
    apps.iter()
        .map(|app| RemovalOutcome {
            app: *app,
            error: uninstall(serial, app.package).err(),
        })
        .collect()
}

/// Uninstalls one package for user 0.
fn uninstall(serial: &str, package: &str) -> Result<(), String> {
    let output = run_adb(&[
        "-s",
        serial,
        "shell",
        "pm",
        "uninstall",
        "--user",
        "0",
        package,
    ])?;

    // `pm uninstall` answers `Success` on stdout and exits 0 either way, so
    // its text is what says whether the app is gone.
    if output.trim() == "Success" {
        Ok(())
    } else if output.trim().is_empty() {
        Err("the device did not report a result".to_owned())
    } else {
        Err(output.trim().to_owned())
    }
}

/// Splits `adb devices -l` output into the devices it lists.
///
/// A line starts with the serial and the state (`device`, `unauthorized`,
/// `offline`, …); the header and the daemon-start notices are not devices.
pub fn parse_devices(output: &str) -> Vec<AdbDevice> {
    output.lines().filter_map(parse_device_line).collect()
}

fn parse_device_line(line: &str) -> Option<AdbDevice> {
    let line = line.trim();

    if line.is_empty() || line.starts_with('*') || line.starts_with("List of devices") {
        return None;
    }

    let mut parts = line.split_whitespace();
    let serial = parts.next()?.to_owned();

    let state = match parts.next()? {
        "device" => AdbDeviceState::Ready,
        "unauthorized" => AdbDeviceState::Unauthorized,
        "offline" => AdbDeviceState::Offline,
        _ => AdbDeviceState::Unknown,
    };

    let model = line
        .split_whitespace()
        .find_map(|token| token.strip_prefix("model:"))
        .map(|model| model.replace('_', " "));

    Some(AdbDevice {
        serial,
        state,
        model,
    })
}

/// The package names in `pm list packages` output, each line of which reads
/// `package:<name>`.
pub fn parse_packages(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("package:"))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The bloatware of [`BLOATWARE`] that is among `packages`, in that order.
pub fn installed(packages: &[String]) -> Vec<Bloatware> {
    BLOATWARE
        .iter()
        .copied()
        .filter(|app| packages.iter().any(|package| package == app.package))
        .collect()
}

/// Whether a row matches what the search box contains.
///
/// The query is looked for, without case, in the app's name and in its package
/// name; an empty query (or one that is only spaces) matches every row.
/// `name` is the name [`Bloatware::label`] resolved for the current language.
pub fn matches(name: &str, package: &str, query: &str) -> bool {
    let query = query.trim();

    if query.is_empty() {
        return true;
    }

    let query = query.to_lowercase();

    name.to_lowercase().contains(&query) || package.to_lowercase().contains(&query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_parsed_with_their_state() {
        // What a real `adb devices -l` prints while the daemon starts.
        let output = "* daemon not running; starting now at tcp:5037\n\
                      * daemon started successfully\n\
                      List of devices attached\n\
                      ZY22ABCXYZ            device product:arcfox model:moto_g84_5G device:arcfox transport_id:1\n\
                      0123456789ABCDEF      unauthorized usb:1-3 transport_id:2\n\
                      9876543210            offline\n";

        let devices = parse_devices(output);

        assert_eq!(devices.len(), 3);

        assert_eq!(devices[0].serial, "ZY22ABCXYZ");
        assert_eq!(devices[0].state, AdbDeviceState::Ready);
        assert_eq!(devices[0].model.as_deref(), Some("moto g84 5G"));
        assert_eq!(devices[0].label(), "ZY22ABCXYZ (moto g84 5G)");

        assert_eq!(devices[1].state, AdbDeviceState::Unauthorized);
        assert_eq!(devices[1].model, None);
        assert_eq!(devices[1].label(), "0123456789ABCDEF");

        assert_eq!(devices[2].state, AdbDeviceState::Offline);
    }

    #[test]
    fn an_empty_device_list_has_no_devices() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn packages_are_parsed_from_the_listing() {
        let output = "package:com.android.settings\n\
                      package:com.zui.browser\n\
                      \n\
                      package:com.lenovo.hyperengine\n";

        assert_eq!(
            parse_packages(output),
            vec![
                "com.android.settings",
                "com.zui.browser",
                "com.lenovo.hyperengine"
            ]
        );
    }

    #[test]
    fn only_the_known_packages_are_installed() {
        let on_device = vec![
            "com.android.settings".to_owned(),
            "com.zui.browser".to_owned(),
            "com.motorola.smartfeed".to_owned(),
        ];

        let found = installed(&on_device);

        // In the order of the table, not of the device's listing.
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].package, "com.zui.browser");
        assert_eq!(found[1].package, "com.motorola.smartfeed");
    }

    #[test]
    fn the_search_looks_at_names_and_packages() {
        let name = "Verizon Messages";
        let package = "com.verizon.messaging.vzmsgs";

        // An empty query keeps every row.
        assert!(matches(name, package, ""));
        assert!(matches(name, package, "   "));

        assert!(matches(name, package, "verizon"));
        assert!(matches(name, package, "MESSAGES"));
        assert!(matches(name, package, "vzmsgs"));

        assert!(!matches(name, package, "sprint"));
    }

    #[test]
    fn every_package_in_the_table_is_unique() {
        for (index, app) in BLOATWARE.iter().enumerate() {
            assert!(
                !BLOATWARE[..index]
                    .iter()
                    .any(|other| other.package == app.package),
                "duplicate package {}",
                app.package
            );

            // A row must always name the app somehow: either the product name
            // or the FTL id its translation lives under.
            match app.label {
                Label::Message(id) => assert!(!id.is_empty(), "{}", app.package),
                Label::Text(name) => assert!(!name.trim().is_empty(), "{}", app.package),
            }
        }
    }
}
