//! The "Smartphone Firmware Flash" mode (Mode 2): a grid of feature tiles.
//!
//! Each tile is one flashing feature and carries the buttons that start it.
//! The feature's flow state lives in `crate` (`State::flash`); this module only
//! describes the grid and which messages its buttons send.

use iced::widget::{button, column, container, scrollable, Row};
use iced::{Alignment, Element, Fill};

use crate::text;
use crate::{driver_install, Message, State};

/// A feature tile offered by the smartphone-firmware-flash mode (Mode 2).
///
/// Each variant maps to a localized title and action button; new flashing
/// features are added here as they are implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmartphoneFeature {
    BootloaderUnlock,
    FactoryReset,
    FirmwareFlash,
    InstallDriver,
}

impl SmartphoneFeature {
    pub(crate) const ALL: [Self; 4] = [
        Self::BootloaderUnlock,
        Self::FactoryReset,
        Self::FirmwareFlash,
        Self::InstallDriver,
    ];

    fn title_id(self) -> &'static str {
        match self {
            Self::BootloaderUnlock => "flash-bootloader-title",
            Self::FactoryReset => "flash-factory-reset-title",
            Self::FirmwareFlash => "flash-firmware-title",
            Self::InstallDriver => "flash-driver-title",
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

    let actions: Vec<Element<'_, Message>> = feature
        .actions()
        .into_iter()
        .map(|(label_id, message)| -> Element<'_, Message> {
            let action = button(text(l10n.tr(label_id))).width(Fill);

            match message {
                Some(message) => action.on_press(message).into(),
                // Not available yet: iced greys out a button without `on_press`
                // and ignores clicks on it.
                None => action.into(),
            }
        })
        .collect();

    container(
        column![
            text(l10n.tr(feature.title_id())).size(16.0),
            Row::with_children(actions).spacing(8),
        ]
        .spacing(12)
        .align_x(Alignment::Start),
    )
    .width(320)
    .padding(16)
    .style(container::rounded_box)
    .into()
}
