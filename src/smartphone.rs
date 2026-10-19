//! The "Smartphone Firmware Flash" mode (Mode 2): a grid of feature tiles.
//!
//! Each tile is one flashing feature and carries the buttons that start it.
//! The feature's flow state lives in `crate` (`State::flash`); this module only
//! describes the grid and which messages its buttons send.

use iced::widget::{button, column, container, scrollable};
use iced::{Alignment, Fill};

use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::text;
use crate::{driver_install, Message, State};

/// The room a tile's action buttons keep around their labels when the tile
/// sizes them to their content instead of sharing its width.
const COMPACT_ACTION_PADDING: [f32; 2] = [3.0, 8.0];

/// A feature tile offered by the smartphone-firmware-flash mode (Mode 2).
///
/// Each variant maps to a localized title and action button; new flashing
/// features are added here as they are implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmartphoneFeature {
    BootloaderUnlock,
    FactoryReset,
    FirmwareFlash,
    PartitionFlash,
    MultiFlashing,
    PartitionManager,
    InstallDriver,
    RemoveBloatware,
    WirelessDebugging,
    FixCaptivePortal,
}

impl SmartphoneFeature {
    pub(crate) const ALL: [Self; 10] = [
        Self::BootloaderUnlock,
        Self::FactoryReset,
        Self::FirmwareFlash,
        Self::PartitionFlash,
        Self::MultiFlashing,
        Self::PartitionManager,
        Self::InstallDriver,
        Self::RemoveBloatware,
        Self::WirelessDebugging,
        Self::FixCaptivePortal,
    ];

    fn title_id(self) -> &'static str {
        match self {
            Self::BootloaderUnlock => "flash-bootloader-title",
            Self::FactoryReset => "flash-factory-reset-title",
            Self::FirmwareFlash => "flash-firmware-title",
            Self::PartitionFlash => "flash-partition-title",
            Self::MultiFlashing => "flash-multi-flashing-title",
            Self::PartitionManager => "flash-partition-manager-title",
            Self::InstallDriver => "flash-driver-title",
            Self::RemoveBloatware => "flash-bloatware-title",
            Self::WirelessDebugging => "flash-wireless-title",
            Self::FixCaptivePortal => "flash-captive-portal-title",
        }
    }

    /// Whether this system has a use for the feature.
    ///
    /// "Install Driver" is left out where there is nothing to install:
    /// macOS ships the drivers it needs, and Motorola has no installer for
    /// Windows on ARM. The tile is not rendered at all there — an unavailable
    /// feature is not advertised with a dead button.
    fn available(self) -> bool {
        match self {
            Self::InstallDriver => driver_install::target().is_some(),
            _ => true,
        }
    }

    /// Whether the tile's action buttons size themselves to their labels
    /// instead of sharing the tile's width.
    ///
    /// The Partition Manager offers three vendors, and an equal third of the
    /// card is narrower than the word "Qualcomm" — the label would bleed out
    /// of its button.
    fn actions_fit_content(self) -> bool {
        matches!(self, Self::PartitionManager)
    }

    /// The action buttons of the tile, paired with the FTL id of their label
    /// and the message they send.
    ///
    /// `None` marks an action that is not implemented yet: its button is
    /// rendered disabled (a button without `on_press`), so the tile already
    /// shows what is coming without pretending to work.
    fn actions(self) -> Vec<(&'static str, Option<Message>)> {
        let pressed = || Some(Message::SmartphoneFeaturePressed(self));

        match self {
            // The bootloader unlock procedure differs between phones and
            // tablets.
            Self::BootloaderUnlock => vec![
                ("flash-bootloader-smartphone-button", pressed()),
                (
                    "flash-bootloader-tablet-button",
                    Some(Message::TabletUnlockSelected),
                ),
            ],
            Self::FactoryReset => vec![("flash-factory-reset-button", pressed())],
            // Firmware flashing is offered per device type; tablet firmware
            // flashing is not implemented yet.
            Self::FirmwareFlash => vec![
                ("flash-firmware-smartphone-button", pressed()),
                ("flash-firmware-tablet-button", None),
            ],
            // One image written to one partition, without a firmware package
            // behind it: the counterpart of flashing a whole package, and the
            // way back to a working phone when a single partition was written
            // wrong.
            Self::PartitionFlash => vec![("flash-partition-button", pressed())],
            // Flashing a whole batch of phones at once has a workspace of its
            // own, which takes over the window. It sits next to the one-device
            // procedure rather than inside it, because a run is set up (and
            // watched) on its own.
            Self::MultiFlashing => vec![(
                "flash-multi-flashing-button",
                Some(Message::MultiFlashingSelected),
            )],
            // The partition manager is a workspace of its own as well: it reads
            // the partition table of a phone through its vendor's download mode
            // and writes to the ranges in it. Only the Qualcomm side exists so
            // far; the other vendors' buttons are drawn disabled until their
            // flow is implemented.
            Self::PartitionManager => vec![
                (
                    "flash-partition-manager-qualcomm-button",
                    Some(Message::QualcommManagerSelected),
                ),
                ("flash-partition-manager-mediatek-button", None),
                ("flash-partition-manager-unisoc-button", None),
            ],
            // Windows installs a driver per device type: phones take
            // Motorola's Mobile Drivers, tablets are flashed with Lenovo's
            // "Software Fix", which is a download page of its own. Linux has a
            // single installation for both: the udev rules.
            Self::InstallDriver => match driver_install::target() {
                Some(driver_install::Target::Windows(_)) => vec![
                    ("driver-smartphone-button", Some(Message::DriverInstallRequested)),
                    ("driver-tablet-button", Some(Message::DriverTabletSite)),
                ],
                _ => vec![(
                    "driver-install-button",
                    Some(Message::DriverInstallRequested),
                )],
            },
            // Removing preinstalled apps works on any Android device that has
            // USB debugging on, phones and tablets alike.
            Self::RemoveBloatware => vec![("flash-bloatware-button", pressed())],
            // Wireless debugging is a phone feature: the connection is paired
            // over Wi-Fi instead of over the cable.
            Self::WirelessDebugging => vec![("flash-wireless-button", pressed())],
            // The captive-portal fix talks to a device over ADB, wired or
            // wireless alike.
            Self::FixCaptivePortal => vec![("flash-captive-portal-button", pressed())],
        }
    }
}

/// The Mode 2 feature grid. Tiles wrap to the next row once they no longer
/// fit.
pub(crate) fn smartphone_flash_view(state: &State) -> Element<'_, Message> {
    let tiles: Vec<Element<'_, Message>> = SmartphoneFeature::ALL
        .iter()
        // A feature this system has no use for is left out completely (see
        // `SmartphoneFeature::available`).
        .filter(|feature| feature.available())
        .map(|&feature| smartphone_feature_tile(state, feature))
        .collect();

    let grid = container(
        Row::with_children(tiles)
            .spacing(12)
            .align_y(Alignment::Start)
            .wrap(),
    )
    .width(Fill)
    .padding(8);

    container(scrollable(grid).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}

/// A single feature tile in the smartphone-flash grid.
fn smartphone_feature_tile(state: &State, feature: SmartphoneFeature) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let fit_content = feature.actions_fit_content();

    let actions: Vec<Element<'_, Message>> = feature
        .actions()
        .into_iter()
        .map(|(label_id, message)| -> Element<'_, Message> {
            let action = button(text(l10n.tr(label_id)));

            let action = if fit_content {
                action.padding(COMPACT_ACTION_PADDING)
            } else {
                action.width(Fill)
            };

            match message {
                Some(message) => action.on_press(message).into(),
                // Not available yet: iced greys out a button without `on_press`
                // and ignores clicks on it.
                None => action.into(),
            }
        })
        .collect();

    // Buttons that size themselves to their labels may not fit one line in
    // every locale, so their row wraps. A wrapping row has no `spacing`, so
    // the spacing is set before `.wrap()`.
    let actions: Element<'_, Message> = if fit_content {
        Row::with_children(actions).spacing(8).wrap().into()
    } else {
        Row::with_children(actions).spacing(8).into()
    };

    container(
        column![
            text(l10n.tr(feature.title_id())).size(16.0),
            actions,
        ]
        .spacing(12)
        .align_x(Alignment::Start),
    )
    .width(320)
    .padding(16)
    .style(container::rounded_box)
    .into()
}
