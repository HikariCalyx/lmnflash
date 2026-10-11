//! The "Firmware Decrypt" mode (Mode 3): a grid of feature tiles.
//!
//! Like the smartphone-flash mode, each tile is one feature and carries the
//! buttons that start it. The feature's flow state lives in `crate`
//! (`State::decrypt` and `State::unpack`); this module only describes the grid
//! and which messages its buttons send.

use iced::widget::{button, column, container, scrollable, Row};
use iced::{Alignment, Element, Fill};

use crate::text;
use crate::{Message, State};

/// A feature tile offered by the firmware-decrypt mode (Mode 3).
///
/// Each variant maps to a localized title and action button; new host-side
/// firmware-file features are added here as they are implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecryptFeature {
    TabletDecrypt,
    UnpackImage,
}

impl DecryptFeature {
    pub(crate) const ALL: [Self; 2] = [Self::TabletDecrypt, Self::UnpackImage];

    fn title_id(self) -> &'static str {
        match self {
            Self::TabletDecrypt => "decrypt-title",
            Self::UnpackImage => "unpack-title",
        }
    }

    /// The action buttons of the tile, paired with the FTL id of their label
    /// and the message they send.
    ///
    /// `None` marks an action that is not implemented yet: its button is
    /// rendered disabled (a button without `on_press`), so the tile already
    /// shows what is coming without pretending to work.
    fn actions(self) -> Vec<(&'static str, Option<Message>)> {
        let pressed = || Some(Message::DecryptFeaturePressed(self));

        match self {
            Self::TabletDecrypt => vec![("decrypt-button", pressed())],
            Self::UnpackImage => vec![("unpack-button", pressed())],
        }
    }
}

/// The Mode 3 feature grid. Tiles wrap to the next row once they no longer
/// fit.
pub(crate) fn view(state: &State) -> Element<'_, Message> {
    let tiles: Vec<Element<'_, Message>> = DecryptFeature::ALL
        .iter()
        .map(|&feature| feature_tile(state, feature))
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

/// A single feature tile in the firmware-decrypt grid.
fn feature_tile(state: &State, feature: DecryptFeature) -> Element<'_, Message> {
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
