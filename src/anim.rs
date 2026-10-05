//! The fade + slide transitions played when the user switches features.
//!
//! iced 0.13 has no animation support of its own: nothing tweens, and a widget
//! cannot be drawn with a reduced opacity. A transition is therefore driven by
//! a time subscription — while one is in flight the app ticks every frame,
//! each tick advances a [`Transition`], and the view renders that frame:
//!
//! * the tab content (Mode 1/2/3) slides in a little from the direction of the
//!   tab move, and fades in from the window background colour. The content
//!   itself cannot be made translucent, so instead a *veil* of the window's
//!   own background colour is drawn over it and faded out;
//! * a dialog that just opened slides up a little while a dimming scrim fades
//!   in underneath it;
//! * a dialog whose card is replaced by another one (a step of a flow, or a
//!   device picker appearing on top of it) slides the new card in from the
//!   side, like a tab;
//! * a dialog that closes drops out of the window while its scrim fades away.
//!   The card cannot be faded, so it is moved fully out of view first —
//!   removing it afterwards cannot be seen. The close itself is held back
//!   until then (see `crate::update`).
//!
//! The slide is drawn by [`Animated`], a small custom widget that records its
//! content shifted (and scaled) and clipped to its own bounds. The renderer
//! exposes both operations, but no built-in widget uses them.

use std::time::Duration;

use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{
    layout, mouse, renderer, Clipboard, Layout, Renderer, Shell, Widget,
};
use iced::widget::{container, Space};
use iced::{
    Background, Color, Element, Fill, Length, Rectangle, Size, Transformation,
    Vector,
};

use crate::Message;

/// How long a transition takes to settle into place.
const SETTLE_MS: f32 = 200.0;

/// How long a transition takes to leave.
const LEAVE_MS: f32 = 180.0;

/// How much progress one frame advances (the period of [`FRAME`]).
const FRAME_MS: f32 = 16.0;

/// The period a running transition is ticked with (~60 frames per second).
pub(crate) const FRAME: Duration = Duration::from_millis(16);

/// Distance the tab content — and a dialog card that replaces another one —
/// starts away from its final position.
const SWITCH_SLIDE: f32 = 20.0;

/// Distance a dialog card starts below its final position when it opens.
const DIALOG_RISE: f32 = 16.0;

/// The scale a dialog card starts at when it opens (it grows to 1.0).
const DIALOG_SCALE: f32 = 0.965;

/// The scale a dialog card shrinks to while it leaves.
const DIALOG_EXIT_SCALE: f32 = 0.8;

/// How far a dialog card drops while it leaves, as a fraction of the area it
/// is drawn in (the whole window). Comfortably more than 1.0, so the card is
/// drawn out of view before it is removed and nothing pops, even in a window
/// that is shorter than the card.
const DIALOG_EXIT_DROP: f32 = 1.3;

/// Opacity of the dimming scrim behind an open dialog.
const SCRIM_ALPHA: f32 = 0.30;

/// The curve a transition applies to its progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Easing {
    /// Quick at first, settling at the end: a card easing into place.
    Out,
    /// Slow at first, accelerating: a card leaving.
    In,
}

/// The appearance of an animated element at one end of a transition.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pose {
    /// Where the content sits, relative to its settled position.
    pub(crate) slide: Vector,
    /// The part of the slide that is relative to the size of the area the
    /// content is drawn in, so a card can drop out of a window of any size.
    pub(crate) slide_fraction: Vector,
    /// How large the content is drawn, around its own centre.
    pub(crate) scale: f32,
    /// The opacity of the veil (the tab fade) or scrim (the dialog dimming)
    /// that belongs to this pose.
    pub(crate) veil: f32,
}

/// The pose of an element that is settled and fully drawn.
fn settled() -> Pose {
    Pose {
        slide: Vector::ZERO,
        slide_fraction: Vector::ZERO,
        scale: 1.0,
        veil: 0.0,
    }
}

/// Where the tab content starts when its tab is entered: slid towards the side
/// the tab came from, and hidden behind a veil of the window background.
pub(crate) fn content_start(forward: bool) -> Pose {
    Pose {
        slide: side(forward),
        veil: 1.0,
        ..settled()
    }
}

/// The settled appearance of the tab content.
pub(crate) fn content_rest() -> Pose {
    settled()
}

/// The settled appearance of a dialog card: in place, behind its scrim.
pub(crate) fn dialog_rest() -> Pose {
    Pose {
        veil: SCRIM_ALPHA,
        ..settled()
    }
}

/// Where a dialog card comes from when it opens.
pub(crate) fn dialog_enter() -> Pose {
    Pose {
        slide: Vector::new(0.0, DIALOG_RISE),
        scale: DIALOG_SCALE,
        ..settled()
    }
}

/// Where a dialog card comes from when another card takes its place inside the
/// same dialog (a step of a flow): it slides in from the side, over the scrim
/// that is already there.
pub(crate) fn dialog_switch(forward: bool) -> Pose {
    Pose {
        slide: side(forward),
        ..dialog_rest()
    }
}

/// Where a dialog card goes when it leaves: dropped out of view and shrunk,
/// with its scrim faded away.
pub(crate) fn dialog_exit() -> Pose {
    Pose {
        slide_fraction: Vector::new(0.0, DIALOG_EXIT_DROP),
        scale: DIALOG_EXIT_SCALE,
        ..settled()
    }
}

/// `+x` when moving forward (the content comes in from the right), `-x` when
/// moving back.
fn side(forward: bool) -> Vector {
    Vector::new(if forward { SWITCH_SLIDE } else { -SWITCH_SLIDE }, 0.0)
}

/// The colour iced clears the window with, which is also the colour of every
/// empty area of the UI.
///
/// A veil that fades content in from the background has to use exactly this
/// colour; anything else would be visible as a tint. The app does not choose a
/// theme, so this is the same [`iced::Theme::default`] the application itself
/// uses (which follows the OS light/dark setting).
pub(crate) fn window_background() -> Color {
    iced::Theme::default().extended_palette().background.base.color
}

/// A transition that eases an element from one [`Pose`] to another.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Transition {
    /// Milliseconds elapsed since the transition started.
    elapsed: f32,
    duration: f32,
    easing: Easing,
    from: Pose,
    to: Pose,
}

impl Transition {
    /// A transition that eases out into `to` (a card settling into place).
    pub(crate) fn settle(from: Pose, to: Pose) -> Self {
        Self {
            elapsed: 0.0,
            duration: SETTLE_MS,
            easing: Easing::Out,
            from,
            to,
        }
    }

    /// A transition that eases in towards `to` (a card leaving).
    pub(crate) fn leave(from: Pose, to: Pose) -> Self {
        Self {
            elapsed: 0.0,
            duration: LEAVE_MS,
            easing: Easing::In,
            from,
            to,
        }
    }

    /// Advances the transition by one frame.
    ///
    /// Returns whether it is still running; a `false` means the element has
    /// reached `to` and the caller can stop ticking (and drop the transition).
    pub(crate) fn advance(&mut self) -> bool {
        self.elapsed += FRAME_MS;
        !self.finished()
    }

    fn finished(&self) -> bool {
        self.elapsed >= self.duration
    }

    /// The linear progress of the transition, in `0.0..=1.0`.
    pub(crate) fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// The progress eased for the direction the transition plays in.
    pub(crate) fn eased(&self) -> f32 {
        let progress = self.progress();

        match self.easing {
            // Cubic ease-out: most of the movement happens up front.
            Easing::Out => 1.0 - (1.0 - progress).powi(3),
            // Cubic ease-in: the movement builds up towards the end.
            Easing::In => progress.powi(3),
        }
    }

    /// The appearance of the element in this frame.
    pub(crate) fn pose(&self) -> Pose {
        let eased = self.eased();

        Pose {
            slide: mix(self.from.slide, self.to.slide, eased),
            slide_fraction: mix(self.from.slide_fraction, self.to.slide_fraction, eased),
            scale: mix_number(self.from.scale, self.to.scale, eased),
            veil: mix_number(self.from.veil, self.to.veil, eased).clamp(0.0, 1.0),
        }
    }
}

fn mix(from: Vector, to: Vector, ratio: f32) -> Vector {
    Vector::new(
        mix_number(from.x, to.x, ratio),
        mix_number(from.y, to.y, ratio),
    )
}

fn mix_number(from: f32, to: f32, ratio: f32) -> f32 {
    from + (to - from) * ratio
}

/// Advances an optional transition by one frame, clearing it once it is done.
pub(crate) fn tick(transition: &mut Option<Transition>) {
    let Some(active) = transition.as_mut() else {
        return;
    };

    if !active.advance() {
        *transition = None;
    }
}

/// A full-size, non-interactive veil of `color` at `alpha`, drawn over the
/// content it hides. Used to fade the tab content in from the window
/// background colour.
pub(crate) fn veil<'a>(color: Color, alpha: f32) -> Element<'a, Message> {
    container(Space::new(Fill, Fill))
        .width(Fill)
        .height(Fill)
        .style(move |_theme| container::Style {
            background: Some(Background::Color(Color { a: alpha, ..color })),
            ..container::Style::default()
        })
        .into()
}

/// A full-size, non-interactive dimming scrim, drawn between the base UI and
/// an open dialog.
pub(crate) fn scrim<'a>(alpha: f32) -> Element<'a, Message> {
    veil(Color::BLACK, alpha)
}

/// Wraps `content` so that it is drawn in `pose`: slid by its slide (absolute,
/// plus a part relative to its own size) and scaled around its own centre,
/// clipped to its bounds.
pub(crate) fn animated<'a>(
    content: impl Into<Element<'a, Message>>,
    pose: Pose,
) -> Element<'a, Message> {
    Element::new(Animated {
        content: content.into(),
        pose,
    })
}

/// Draws its content posed, without moving it in the layout.
///
/// The content keeps its unposed layout, so it is hit-tested where it would be
/// if the transition were over. During the ~0.2 s a transition lasts that is
/// imperceptible, and it means the widget never has to translate the cursor
/// back.
struct Animated<'a> {
    content: Element<'a, Message>,
    pose: Pose,
}

impl Widget<Message, iced::Theme, iced::Renderer> for Animated<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget().layout(
            &mut tree.children[0],
            renderer,
            limits,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let center = bounds.center();
        let pose = self.pose;

        // `Transformation::scale` scales around the origin of the window, so
        // the content's centre is kept in place by adding the difference.
        let offset = Vector::new(
            pose.slide.x
                + pose.slide_fraction.x * bounds.width
                + center.x * (1.0 - pose.scale),
            pose.slide.y
                + pose.slide_fraction.y * bounds.height
                + center.y * (1.0 - pose.scale),
        );

        renderer.with_layer(bounds, |renderer| {
            renderer.with_transformation(
                Transformation::translate(offset.x, offset.y)
                    * Transformation::scale(pose.scale),
                |renderer| {
                    self.content.as_widget().draw(
                        &tree.children[0],
                        renderer,
                        theme,
                        style,
                        layout,
                        cursor,
                        viewport,
                    );
                },
            );
        });
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
    }

    fn state(&self) -> tree::State {
        tree::State::None
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget().operate(
            &mut tree.children[0],
            layout,
            renderer,
            operation,
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> iced::event::Status {
        self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>>
    {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transition_runs_for_its_duration_and_settles() {
        let mut transition = Transition::settle(content_start(true), content_rest());

        assert_eq!(transition.progress(), 0.0);
        assert_eq!(transition.pose().slide, Vector::new(SWITCH_SLIDE, 0.0));

        let mut frames = 0;
        while transition.advance() {
            frames += 1;
            assert!(frames < 1_000, "the transition never finished");
        }

        assert_eq!(transition.progress(), 1.0);
        assert_eq!(transition.pose().slide, Vector::ZERO);
        assert!(transition.pose().veil < f32::EPSILON);
    }

    #[test]
    fn the_content_slide_only_ever_moves_towards_its_place() {
        let mut transition = Transition::settle(content_start(false), content_rest());
        let mut previous = transition.pose().slide.x.abs();

        while transition.advance() {
            let current = transition.pose().slide.x.abs();
            assert!(current <= previous, "the slide grew back");
            previous = current;
        }

        // The last frame is still a little short of the place, and the frame
        // after it snaps there.
        assert!(previous < SWITCH_SLIDE * 0.01);
        assert_eq!(transition.pose().slide, Vector::ZERO);
    }

    #[test]
    fn a_settling_transition_is_eased_out_and_a_leaving_one_is_eased_in() {
        let mut settling = Transition::settle(content_start(true), content_rest());
        settling.elapsed = SETTLE_MS / 2.0;
        // Ease-out front-loads the movement: half the time, more than half the
        // distance.
        assert!(settling.eased() > 0.5);
        assert!(settling.pose().slide.x < SWITCH_SLIDE / 2.0);

        let mut leaving = Transition::leave(dialog_rest(), dialog_exit());
        leaving.elapsed = LEAVE_MS / 2.0;
        // Ease-in back-loads it: half the time, less than half the way.
        assert!(leaving.eased() < 0.5);
        assert!(leaving.pose().slide_fraction.y < DIALOG_EXIT_DROP / 2.0);
    }

    #[test]
    fn a_leaving_dialog_ends_out_of_view_and_undimmed() {
        let mut transition = Transition::leave(dialog_rest(), dialog_exit());

        assert_eq!(transition.pose().veil, SCRIM_ALPHA);

        while transition.advance() {}

        let pose = transition.pose();
        // Dropped fully out of the area it is drawn in (so removing it cannot
        // be seen), with the scrim gone with it.
        assert!(pose.slide_fraction.y >= DIALOG_EXIT_DROP);
        assert!(pose.veil < f32::EPSILON);
    }

    #[test]
    fn ticking_clears_a_finished_transition() {
        let mut transition =
            Some(Transition::settle(content_start(true), content_rest()));
        let frames = (SETTLE_MS / FRAME_MS) as usize + 1;

        for _ in 0..frames {
            assert!(transition.is_some());
            tick(&mut transition);
        }

        assert!(transition.is_none());
    }
}
